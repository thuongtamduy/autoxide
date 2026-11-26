- A tenant domain setting (`auth.passkey_rp_id`) also refuses a name whose last label is a hex number
  (`shop.0x7f`), which browsers read as an IPv4 address; the error message says IP addresses are not
  accepted.
