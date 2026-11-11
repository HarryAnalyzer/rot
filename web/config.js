// Which contracts the page talks to.
//
// Two deployments of the same wasm, differing only in the length of a "day":
//
//   demo  - a day is one minute, so the whole economy plays out while you watch
//   slow  - a day is a day, the cadence the simulation was tuned for
//
// Override either from the URL without editing anything:
//   index.html?contract=rot.someone.testnet&network=testnet
//
// scripts/deploy.ps1 rewrites the two ids below after it deploys.

window.ROT_CONFIG = {
  deployments: {
    demo: {
      label: "demo - 1 minute per day",
      contract: "rot-demo.worgl.testnet",
      network: "testnet",
      blurb:
        "Same contract, same rules, but a day lasts a minute. Rot, epochs " +
        "and payouts all happen while you watch.",
    },
    slow: {
      label: "real - 1 day per day",
      contract: "rot.worgl.testnet",
      network: "testnet",
      blurb:
        "The cadence the parameters were tuned for: 2% a day, 7-day stamps, " +
        "7-day epochs.",
    },
  },
  default: "demo",
  rpc: {
    testnet: ["https://test.rpc.fastnear.com", "https://rpc.testnet.near.org"],
    mainnet: ["https://free.rpc.fastnear.com", "https://rpc.mainnet.near.org"],
  },
  explorer: {
    testnet: "https://testnet.nearblocks.io",
    mainnet: "https://nearblocks.io",
  },
};
