// Filled in during setup (SETUP.md step 7). These values are public: they're in every buyer's browser.
window.FILESCAPE = {
  paddleEnvironment: "production",    // "sandbox" when testing with a Paddle sandbox account
  paddleClientToken: "live_21eecce2369a6cde3fe1226399d", // "Filescape production web" (Paddle > Developer tools > Authentication)
  // Filescape Professional, one-time (product pro_01m84qtt7s6m2r6a44h9t6wrt0). Prices:
  //   standard $9.99  pri_01m86kecyaqrsp7d702v4hc679
  //   launch   $4.99  pri_01m86kf4h9r8tj5p6rtgez78v6  (in use now)
  // To end the launch price: priceId = the standard price, priceLabel "$9.99", listPriceLabel "".
  priceId: "pri_01m86kf4h9r8tj5p6rtgez78v6",
  priceLabel: "$4.99",                // shown before Paddle's checkout gives the exact price with tax
  listPriceLabel: "$9.99",            // shown struck through before priceLabel; "" for none
  successUrl: "https://account.filescape.app/thanks",
};
