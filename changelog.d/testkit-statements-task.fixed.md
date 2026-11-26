- `autoxide-testkit`: `TestApp::statements` counts only the SQL statements of the measured work, never those of a
  background task that runs at the same moment, so a statement-count test no longer turns red at random.
