//! Applying transactions to the UTXO set — the ledger's state-transition rules.
//!
//! Two layers live here:
//!
//! * [`apply_block`] — the strict, atomic state transition for one block's
//!   worth of transactions against a [`UtxoSet`]. It validates every spend
//!   (existence, no double-spend within the block, signature, value
//!   conservation) and the coinbase (issuance ≤ subsidy + fees), then commits.
//!   On *any* error the UTXO set is left untouched.
//! * [`apply_dag`] — the bridge to consensus. It walks
//!   `kovanica_dag::Dag::linearize()` (the deterministic GHOSTDAG total order),
//!   decodes each block's payload into transactions, and applies them in that
//!   order. This is what makes the ledger a *DAG* ledger: parallel blocks are
//!   ordered by GHOSTDAG, and a conflicting spend loses simply because the block
//!   that wins the linearization spent the output first.
//!
//! ## Rules, precisely
//!
//! For a non-coinbase transaction against the current UTXO set:
//! 1. it has at least one input and one output;
//! 2. no outpoint is spent twice within the same transaction;
//! 3. every spent outpoint is currently unspent (exists in the set);
//! 4. every input's signature verifies against the spent output's owner over the
//!    transaction's [`sighash`](crate::tx::Transaction::sighash);
//! 5. every output value is non-zero;
//! 6. outputs do not exceed inputs; the difference is the fee.
//!
//! A block may begin with a single coinbase (input-less) transaction. Regular
//! transactions are applied first, in list order, accumulating fees; then the
//! coinbase's outputs are validated to sum to **at most** `subsidy + fees` and
//! applied last. Because the coinbase is applied last, its outputs are not
//! spendable within the same block (a light-touch maturity rule).
//!
//! ## Deliberate first-slice simplifications
//!
//! * `subsidy` is a single per-block constant passed in, not a halving schedule.
//! * No coinbase maturity beyond "not in the same block"; no fee floor; no tx
//!   size/weight limits. These belong with the pruning/finality slice.
//! * [`apply_dag`] applies against a fresh state each call — there is no
//!   incremental re-org handling yet (the GHOSTDAG order is taken as a snapshot).

use std::collections::HashSet;

use kovanica_dag::{BlockId, Dag};

use crate::keys::verify;
use crate::tx::{decode_block_payload, DecodeError, OutPoint, Transaction, TxId};
use crate::utxo::UtxoSet;

/// Why a transaction or block could not be applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerError {
    /// A spent outpoint is not in the UTXO set (missing, already spent, or —
    /// for a same-block coinbase output — not yet mature).
    MissingInput(OutPoint),
    /// The same outpoint is spent twice within one transaction.
    DuplicateInput(OutPoint),
    /// A created output's outpoint already exists (id collision / replay).
    OutputAlreadyExists(OutPoint),
    /// An input's signature did not verify against the spent output's owner.
    BadSignature { tx: TxId, input: usize },
    /// A transaction's outputs exceed its inputs (it would mint value).
    ValueNotConserved { tx: TxId, inputs: u64, outputs: u64 },
    /// A value sum overflowed `u64`.
    ValueOverflow,
    /// A non-coinbase transaction had no inputs, or a coinbase had no outputs to
    /// carry any value — i.e. a structurally empty transaction where the model
    /// requires content.
    EmptyTransaction(TxId),
    /// An output has zero value.
    ZeroValueOutput(TxId),
    /// A coinbase (input-less) transaction appeared somewhere other than first.
    MisplacedCoinbase(TxId),
    /// The coinbase claims more than `subsidy + fees` allows.
    CoinbaseOverspend { claimed: u64, allowed: u64 },
    /// A block's payload could not be decoded into transactions.
    Payload(DecodeError),
}

impl core::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LedgerError::MissingInput(op) => write!(f, "missing/spent input {op:?}"),
            LedgerError::DuplicateInput(op) => write!(f, "duplicate input {op:?}"),
            LedgerError::OutputAlreadyExists(op) => write!(f, "output already exists {op:?}"),
            LedgerError::BadSignature { tx, input } => {
                write!(f, "bad signature on input {input} of {tx}")
            }
            LedgerError::ValueNotConserved {
                tx,
                inputs,
                outputs,
            } => write!(
                f,
                "value not conserved in {tx}: in {inputs} < out {outputs}"
            ),
            LedgerError::ValueOverflow => f.write_str("value sum overflowed"),
            LedgerError::EmptyTransaction(tx) => write!(f, "empty transaction {tx}"),
            LedgerError::ZeroValueOutput(tx) => write!(f, "zero-value output in {tx}"),
            LedgerError::MisplacedCoinbase(tx) => write!(f, "misplaced coinbase {tx}"),
            LedgerError::CoinbaseOverspend { claimed, allowed } => {
                write!(
                    f,
                    "coinbase overspend: claimed {claimed} > allowed {allowed}"
                )
            }
            LedgerError::Payload(e) => write!(f, "payload decode: {e}"),
        }
    }
}

impl std::error::Error for LedgerError {}

/// What a successfully applied block moved: total fees collected from its
/// transactions and total value minted by its coinbase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BlockSummary {
    /// Sum of `inputs − outputs` across the block's non-coinbase transactions.
    pub fees: u64,
    /// Total value claimed by the block's coinbase (0 if none).
    pub minted: u64,
}

/// Apply one block's transactions to `utxo`, atomically.
///
/// `txs` is the block's transaction list; if the first has no inputs it is the
/// coinbase. `subsidy` is the issuance allowance for this block. Returns a
/// [`BlockSummary`] on success. On any error, `utxo` is left exactly as it was.
pub fn apply_block(
    utxo: &mut UtxoSet,
    txs: &[Transaction],
    subsidy: u64,
) -> Result<BlockSummary, LedgerError> {
    // Stage all changes on a copy; only commit if the whole block validates, so
    // a rejected block has no effect (atomicity).
    let mut staging = utxo.clone();

    let mut coinbase: Option<&Transaction> = None;
    let mut total_fees: u64 = 0;

    for (i, tx) in txs.iter().enumerate() {
        if tx.is_coinbase() {
            if i != 0 {
                return Err(LedgerError::MisplacedCoinbase(tx.id()));
            }
            coinbase = Some(tx);
            continue; // applied last, after fees are known
        }
        let fee = apply_regular(&mut staging, tx)?;
        total_fees = total_fees
            .checked_add(fee)
            .ok_or(LedgerError::ValueOverflow)?;
    }

    let allowed = subsidy
        .checked_add(total_fees)
        .ok_or(LedgerError::ValueOverflow)?;
    let minted = match coinbase {
        Some(cb) => apply_coinbase(&mut staging, cb, allowed)?,
        None => 0,
    };

    *utxo = staging;
    Ok(BlockSummary {
        fees: total_fees,
        minted,
    })
}

/// Validate and apply a regular (non-coinbase) transaction, returning its fee.
fn apply_regular(staging: &mut UtxoSet, tx: &Transaction) -> Result<u64, LedgerError> {
    if tx.inputs().is_empty() || tx.outputs().is_empty() {
        return Err(LedgerError::EmptyTransaction(tx.id()));
    }

    let sighash = tx.sighash();
    let mut seen: HashSet<OutPoint> = HashSet::with_capacity(tx.inputs().len());
    let mut sum_in: u64 = 0;
    for (i, input) in tx.inputs().iter().enumerate() {
        if !seen.insert(input.outpoint) {
            return Err(LedgerError::DuplicateInput(input.outpoint));
        }
        let prev = staging
            .get(&input.outpoint)
            .ok_or(LedgerError::MissingInput(input.outpoint))?;
        if !verify(&prev.owner, &sighash, &input.signature.to_bytes()) {
            return Err(LedgerError::BadSignature {
                tx: tx.id(),
                input: i,
            });
        }
        sum_in = sum_in
            .checked_add(prev.value)
            .ok_or(LedgerError::ValueOverflow)?;
    }

    let mut sum_out: u64 = 0;
    for output in tx.outputs() {
        if output.value == 0 {
            return Err(LedgerError::ZeroValueOutput(tx.id()));
        }
        sum_out = sum_out
            .checked_add(output.value)
            .ok_or(LedgerError::ValueOverflow)?;
    }

    if sum_out > sum_in {
        return Err(LedgerError::ValueNotConserved {
            tx: tx.id(),
            inputs: sum_in,
            outputs: sum_out,
        });
    }

    // Validation passed; mutate the staging set. (Any error above returned
    // before this point, so partial mutation cannot leak — and `apply_block`
    // discards `staging` unless the whole block succeeds.)
    let txid = tx.id();
    for input in tx.inputs() {
        staging.remove(&input.outpoint);
    }
    add_outputs(staging, txid, tx)?;

    Ok(sum_in - sum_out)
}

/// Validate and apply a coinbase transaction, returning the value minted.
fn apply_coinbase(
    staging: &mut UtxoSet,
    cb: &Transaction,
    allowed: u64,
) -> Result<u64, LedgerError> {
    let mut claimed: u64 = 0;
    for output in cb.outputs() {
        if output.value == 0 {
            return Err(LedgerError::ZeroValueOutput(cb.id()));
        }
        claimed = claimed
            .checked_add(output.value)
            .ok_or(LedgerError::ValueOverflow)?;
    }
    if claimed > allowed {
        return Err(LedgerError::CoinbaseOverspend { claimed, allowed });
    }
    add_outputs(staging, cb.id(), cb)?;
    Ok(claimed)
}

/// Insert every output of `tx` into `staging`, keyed by `(txid, index)`,
/// rejecting any outpoint that already exists.
fn add_outputs(staging: &mut UtxoSet, txid: TxId, tx: &Transaction) -> Result<(), LedgerError> {
    for (i, output) in tx.outputs().iter().enumerate() {
        let outpoint = OutPoint::new(txid, i as u32);
        if staging.insert(outpoint, *output).is_some() {
            return Err(LedgerError::OutputAlreadyExists(outpoint));
        }
    }
    Ok(())
}

/// The result of applying a whole DAG's worth of blocks in linearized order.
#[derive(Clone, Debug, Default)]
pub struct LedgerRun {
    /// The final UTXO set after applying every accepted block.
    pub utxo: UtxoSet,
    /// Blocks that applied cleanly, in linearization order.
    pub accepted: Vec<BlockId>,
    /// Blocks rejected as invalid, each with the reason. A block is rejected —
    /// not fatal — so a conflicting/invalid block never halts the ledger; it
    /// simply has no effect. Determined entirely by the GHOSTDAG order.
    pub rejected: Vec<(BlockId, LedgerError)>,
}

/// Apply an entire DAG: linearize it with GHOSTDAG, then apply each block's
/// transactions in that order against a fresh UTXO set.
///
/// This is a pure function of the DAG and `subsidy`: the linearization is
/// deterministic, and so is every state transition, so two nodes holding the
/// same DAG derive the identical [`LedgerRun`]. Conflicting spends across
/// parallel blocks are resolved by the linearization — the earlier block spends
/// the output; the later one is rejected with [`LedgerError::MissingInput`].
pub fn apply_dag(dag: &Dag, subsidy: u64) -> LedgerRun {
    let mut run = LedgerRun::default();
    for id in dag.linearize() {
        let payload = dag
            .block(&id)
            .expect("linearized id is present in the DAG")
            .payload();
        match decode_block_payload(payload) {
            Ok(txs) => match apply_block(&mut run.utxo, &txs, subsidy) {
                Ok(_) => run.accepted.push(id),
                Err(e) => run.rejected.push((id, e)),
            },
            Err(e) => run.rejected.push((id, LedgerError::Payload(e))),
        }
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::KeyPair;
    use crate::tx::{TxId, TxOutput};

    // A funded outpoint owned by `kp`, seeded directly into a UTXO set so unit
    // tests need no coinbase plumbing.
    fn funded(set: &mut UtxoSet, kp: &KeyPair, value: u64, seed: u8) -> OutPoint {
        let op = OutPoint::new(TxId::from_bytes([seed; 32]), 0);
        set.insert(op, TxOutput::new(value, kp.address()));
        op
    }

    #[test]
    fn transfer_conserves_value_and_pays_fee() {
        let alice = KeyPair::from_u64(1);
        let bob = KeyPair::from_u64(2);
        let mut utxo = UtxoSet::new();
        let op = funded(&mut utxo, &alice, 100, 1);

        let tx = Transaction::signed(
            &[(op, &alice)],
            vec![TxOutput::new(90, bob.address())],
            vec![],
        );
        let summary = apply_block(&mut utxo, &[tx], 0).unwrap();

        assert_eq!(summary.fees, 10); // 100 in − 90 out
        assert_eq!(utxo.balance(&bob.address()), 90);
        assert_eq!(utxo.balance(&alice.address()), 0);
        assert!(!utxo.contains(&op), "spent input is gone");
    }

    #[test]
    fn bad_signature_is_rejected_and_atomic() {
        let alice = KeyPair::from_u64(1);
        let mallory = KeyPair::from_u64(9);
        let bob = KeyPair::from_u64(2);
        let mut utxo = UtxoSet::new();
        let op = funded(&mut utxo, &alice, 100, 1);
        let before = utxo.total_value();

        // Mallory signs a spend of Alice's output.
        let tx = Transaction::signed(
            &[(op, &mallory)],
            vec![TxOutput::new(50, bob.address())],
            vec![],
        );
        let err = apply_block(&mut utxo, &[tx], 0).unwrap_err();

        assert!(matches!(err, LedgerError::BadSignature { input: 0, .. }));
        assert_eq!(
            utxo.total_value(),
            before,
            "rejected block left state untouched"
        );
        assert!(utxo.contains(&op));
    }

    #[test]
    fn overspend_is_rejected() {
        let alice = KeyPair::from_u64(1);
        let bob = KeyPair::from_u64(2);
        let mut utxo = UtxoSet::new();
        let op = funded(&mut utxo, &alice, 100, 1);

        let tx = Transaction::signed(
            &[(op, &alice)],
            vec![TxOutput::new(101, bob.address())],
            vec![],
        );
        let err = apply_block(&mut utxo, &[tx], 0).unwrap_err();
        assert!(matches!(
            err,
            LedgerError::ValueNotConserved {
                inputs: 100,
                outputs: 101,
                ..
            }
        ));
    }

    #[test]
    fn double_spend_within_block_is_rejected() {
        let alice = KeyPair::from_u64(1);
        let bob = KeyPair::from_u64(2);
        let mut utxo = UtxoSet::new();
        let op = funded(&mut utxo, &alice, 100, 1);

        let first = Transaction::signed(
            &[(op, &alice)],
            vec![TxOutput::new(90, bob.address())],
            b"1".to_vec(),
        );
        // Second tx spends the same outpoint; by application time it's gone.
        let second = Transaction::signed(
            &[(op, &alice)],
            vec![TxOutput::new(80, bob.address())],
            b"2".to_vec(),
        );
        let err = apply_block(&mut utxo, &[first, second], 0).unwrap_err();
        assert_eq!(err, LedgerError::MissingInput(op));
        // Atomic: neither spend took effect.
        assert!(utxo.contains(&op));
    }

    #[test]
    fn coinbase_may_claim_subsidy_plus_fees_but_no_more() {
        let alice = KeyPair::from_u64(1);
        let bob = KeyPair::from_u64(2);
        let miner = KeyPair::from_u64(3);
        let mut utxo = UtxoSet::new();
        let op = funded(&mut utxo, &alice, 100, 1);

        // Transfer leaves a fee of 10; subsidy 50 ⇒ coinbase may claim 60.
        let transfer = Transaction::signed(
            &[(op, &alice)],
            vec![TxOutput::new(90, bob.address())],
            vec![],
        );
        let good_cb =
            Transaction::coinbase(vec![TxOutput::new(60, miner.address())], b"h1".to_vec());
        let summary = apply_block(&mut utxo, &[good_cb, transfer.clone()], 50).unwrap();
        assert_eq!(
            summary,
            BlockSummary {
                fees: 10,
                minted: 60
            }
        );

        // Claiming 61 overspends.
        let mut utxo2 = UtxoSet::new();
        let op2 = funded(&mut utxo2, &alice, 100, 1);
        let transfer2 = Transaction::signed(
            &[(op2, &alice)],
            vec![TxOutput::new(90, bob.address())],
            vec![],
        );
        let greedy_cb =
            Transaction::coinbase(vec![TxOutput::new(61, miner.address())], b"h1".to_vec());
        let err = apply_block(&mut utxo2, &[greedy_cb, transfer2], 50).unwrap_err();
        assert_eq!(
            err,
            LedgerError::CoinbaseOverspend {
                claimed: 61,
                allowed: 60
            }
        );
    }
}
