// Filled in during setup (SETUP.md step 6). These values are public: they're in every buyer's browser.
window.FILESCAPE = {
  paddleEnvironment: "production",    // "sandbox" when testing with a Paddle sandbox account
  paddleClientToken: "live_21eecce2369a6cde3fe1226399d", // "Filescape production web" (Paddle > Developer tools > Authentication)
  priceId: "pri_01m84qwgg1a8858drzqk2rvzpz", // Filescape Professional, one-time (product pro_01m84qtt7s6m2r6a44h9t6wrt0)
  priceLabel: "$5",                   // shown before Paddle's checkout gives the exact price with tax
  downloadWindows: "https://downloads.filescape.app/latest/Filescape-windows.zip",
  downloadMac: "https://downloads.filescape.app/latest/Filescape-macos.dmg",
  successUrl: "https://account.filescape.app/thanks",
};
