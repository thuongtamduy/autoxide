- Module `promotions`: pausing and ending (or two activations of) the same promotion at the same
  time no longer both succeed from the old status — the loser gets `409` instead of a `200` that
  recorded a version and an event (and released the plan slot) for a transition that did not
  happen (Postgres). Edits and deletes read the status under the same lock.
