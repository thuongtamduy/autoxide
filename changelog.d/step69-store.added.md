- In-app purchases (step 69): trait `StorePurchases` in the kernel for App Store and Google Play
  (verify a purchase, receive signed server notifications) with `FakeStorePurchases` in the
  testkit. Module `plans` links store products to plans, takes purchases from the app
  (`POST /api/v1/plans/store/purchases`) and store notifications
  (`POST /api/v1/plans/store/notifications/{store}`): renewals extend the subscription, refunds end
  access at once, the same purchase or notification never grants twice. Apple and Google collect
  the money; real store connectors need the owner's developer accounts.
