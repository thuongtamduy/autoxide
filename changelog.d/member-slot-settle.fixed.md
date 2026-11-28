- Module `auth`: the `auth.members` plan limit no longer drifts when two role changes for the same person run at the
  same time (one removing every role, one adding); after a change the slot follows whether the person really holds a
  role.
