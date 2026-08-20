module.exports = {
  apps: [
    {
      name: "kovanica-explorer",
      cwd: "/root/kovanica-ledger",
      script: "./target/release/kovanica-node",
      args: "explorer 127.0.0.1:8080",
      interpreter: "none",
      autorestart: true,
      max_restarts: 20,
      env: {
        KOVANICA_MINE: "0",
        KOVANICA_FAUCET: "0",
        KOVANICA_ALLOW_RESET: "0",
        KOVANICA_OPERATOR: "0",
        KOVANICA_LISTEN: "0.0.0.0:9000",
      },
    },
  ],
};
