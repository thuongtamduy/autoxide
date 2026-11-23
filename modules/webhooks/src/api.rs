//! Contract of the `webhooks` module for other modules: nothing of its own. The module receives every
//! event through `Module::subscriptions` and delivers it outward; other modules need not call it.
