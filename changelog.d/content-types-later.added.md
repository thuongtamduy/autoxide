- `content`: custom content types, second part — a routable type can have a ready-made entry page
  (`detail_layout`, with block `content.entry_field` showing a field of the entry being viewed; `GET /public/resolve`
  returns it as `detail`); block `content.type_list` lists a type's published entries as cards on any page;
  `GET /public/types/{key}/entries` filters on published values of public filterable fields with
  `cf.<key>=<op>:<value>`; `POST /api/v1/content/types/ai/suggest` drafts a type and its fields with AI (nothing is
  saved); plan feature and limit `content.custom_types`.
