- Module `promotions`: approving and rejecting a seller's application at the same time no longer
  leaves flash-sale slots for a rejected application; a seller cannot edit an application while it
  is being reviewed.
- Module `promotions`: two promotions activated at the same time can no longer both pass the legal
  limit on discount days per year; a promotion changed while it was being checked is refused with
  `promotions.changed_while_activating` (409) instead of being turned on unchecked.
