- Module `automations`: saving, editing or enabling a rule whose steps need a `money` or
  `destructive` permission now requires a recent re-confirmation (`step_up_required`), and so does
  approving such a step; a rule triggered by an event requires the permission to stream that event,
  like webhooks.
