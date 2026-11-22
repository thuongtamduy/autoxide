- Module `shipping`: carrier quotes carry the recipient and the parcel's size when the cart has them;
  GHN quotes no longer send placeholder recipient values once the buyer entered a name and phone.
- Module `notifications`: a message whose provider account the tenant has not set up yet
  (`provider.not_configured`) is skipped with the reason `channel_disabled` instead of failing.
