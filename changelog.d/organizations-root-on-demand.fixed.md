- Module `organizations`: on a single-tenant install the root unit is created the first time another module asks for
  it, so e.g. opening the ledger book of the shop (`POST /api/v1/ledger/books`) no longer fails with "there is no
  organization unit with this id" until someone has used the organizations API.
