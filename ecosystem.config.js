module.exports = {
  apps: [
    {
      name: "kovanica",
      script: "./target/release/kovanica-node",
      cwd: "/home/BetterCallDzuks/kovanica-ledger",
      instances: 1,
      autorestart: true,
      watch: false,
      max_memory_restart: "1G",
      env: {
        RUST_LOG: "info",
      },
      // Optional: log files
      error_file: "./logs/kovanica-error.log",
      out_file: "./logs/kovanica-out.log",
      log_date_format: "YYYY-MM-DD HH:mm:ss",
    },
  ],
};
