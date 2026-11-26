- Automation runs now follow their rule: editing a rule (even only renaming it) or enabling it again stops its runs
  still in progress or waiting, with `automations.rule_changed`, and deleting a rule ends its waiting runs with
  `automations.rule_not_active`. Before, a run kept the rule as it was when it started and finished it. Start the rule
  again if a stopped run was still needed.
