- `menus.default_locale` and `content.default_locale` must be one of the tenant's languages (`menus.locales` /
  `content.locales`, or the server's list when unset) when saved, and those lists must keep a set default:
  422 instead of being skipped when used. To change the language set, remove the default, change the list, then
  set the default again. Values stored before keep the old fallback.
