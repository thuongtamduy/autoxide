# Syncing staff from your identity provider (SCIM 2.0)

Vietnamese: [`scim.vi.md`](scim.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`). They run as tests against every build, so they match the server of this version.

A company that keeps its people in Microsoft Entra ID, Okta or Google Workspace can let that system manage their Autoxide accounts over [SCIM 2.0](https://www.rfc-editor.org/rfc/rfc7644): someone added there gets a staff account here; someone who leaves is disabled at once — their sessions, refresh tokens and API keys are revoked in the same step, with no one having to remember.

The SCIM endpoint is `/api/v1/auth/scim/v2` (`Users`, `Groups`, `ServiceProviderConfig`, `ResourceTypes`, `Schemas`). It follows the SCIM standard, not the usual Autoxide conventions: bodies are `application/scim+json`, lists are `ListResponse` with `startIndex` / `count`, and errors use the SCIM error schema (`status`, `scimType`, `detail`).

## 1. Create a SCIM token

Needs the permission `auth.scim.manage`, a plan with the feature `auth.scim`, and a signed-in session: an API key cannot create one (`auth.session_required`). The token belongs to the tenant (not to a person), works only on the SCIM endpoint, and is shown once.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
```

<!-- example: create-token -->
```bash
curl -sS "$API/api/v1/auth/scim/tokens" -H "Authorization: Bearer $TOKEN" \
  --json '{"name":"Microsoft Entra ID"}'
# expect: 201
# save: SCIM=.data.token_secret
```

The response also has `scim_base_path` (`/api/v1/auth/scim/v2`): put your server's public address in front of it for the provider. List tokens with `GET /api/v1/auth/scim/tokens` (never with their secret). To rotate, create a new token, switch the provider to it, then `DELETE /api/v1/auth/scim/tokens/{id}` the old one. A tenant has at most 5.

## 2. Set up the provider

- **Microsoft Entra ID** — *Enterprise applications → New application → Create your own application* (not in the gallery). Under *Provisioning*, choose *Automatic*; *Tenant URL* `https://api.example.com/api/v1/auth/scim/v2`, *Secret Token* the SCIM token; *Test Connection*. In *Mappings*, keep `userName` ← `userPrincipalName`, map `emails[type eq "work"].value` ← `mail`, and prefer `externalId` ← `objectId`. Entra disables people with `PATCH active=false` and deletes them later.
- **Okta** — open the app integration, *General → Provisioning: SCIM*. In *Provisioning → Integration*: *SCIM connector base URL* as above, *Unique identifier field for users* `userName`, actions *Push New Users*, *Push Profile Updates* and *Push Groups*, *Authentication Mode* *HTTP Header* with the SCIM token. Under *To App*, turn on *Create Users*, *Update User Attributes* and *Deactivate Users*.
- **Google Workspace** — automatic provisioning only reaches apps in Google's catalog. Until Autoxide is listed there, use a SCIM bridge that reads Google Workspace, or let staff sign in with Google ([`oidc.md`](oidc.md)) and give new accounts a role there.

Check what the server supports (no token needed):

<!-- example: service-provider-config -->
```bash
curl -sS "$API/api/v1/auth/scim/v2/ServiceProviderConfig"
```

## 3. What each call does

| Provider call | In Autoxide |
| --- | --- |
| `GET /Users?filter=userName eq "…"` | Finds the linked account (`userName` without case; `externalId` with case). |
| `POST /Users` | Creates a staff account without a password — the person signs in with your sign-in provider or sets a password by reset. An existing staff account with the same email, not yet linked, is taken over instead of duplicated. |
| `PUT` / `PATCH /Users/{id}` | Changes `userName`, `externalId`, the name and the work email (treated as verified). Never the password or two-step sign-in. |
| `active: false` | Disables the account and, in the same transaction, revokes every session, refresh token and API key. Turning it back on does not bring the API keys back. The last active administrator cannot be disabled (409). |
| `DELETE /Users/{id}` | Deletes the account like any account deletion; other modules delete what they keep about the person. |

Example calls, as a provider makes them:

<!-- example: create-user -->
```bash
curl -sS "$API/api/v1/auth/scim/v2/Users" -H "Authorization: Bearer $SCIM" \
  --json '{"schemas":["urn:ietf:params:scim:schemas:core:2.0:User"],"userName":"grace@example.com","externalId":"8f2c1e","name":{"givenName":"Grace","familyName":"Hopper"},"emails":[{"value":"grace@example.com","type":"work","primary":true}],"active":true}'
# expect: 201
# save: USER=.id
```

<!-- example: find-user -->
```bash
curl -sS "$API/api/v1/auth/scim/v2/Users?filter=userName%20eq%20%22Grace%40example.com%22" -H "Authorization: Bearer $SCIM"
```

<!-- example: disable-user -->
```bash
curl -sS -X PATCH "$API/api/v1/auth/scim/v2/Users/$USER" -H "Authorization: Bearer $SCIM" \
  --json '{"schemas":["urn:ietf:params:scim:api:messages:2.0:PatchOp"],"Operations":[{"op":"replace","path":"active","value":false}]}'
```

Every change is in the audit log with the actor kind `scim`.

## 4. Groups and roles

The provider can also push its groups (`/Groups`; Okta: *Push Groups*, Entra ID: assign groups to the app). A group gives nothing by itself: the tenant decides what it means with **mapping rules** — "members of group X hold role R at unit N".

1. Let the provider push the group, then find its id with `GET /api/v1/auth/scim/groups` (member count, rule count). Groups without a rule also show up in the work inbox (`auth.scim_unmapped_groups`).
2. Add a rule with `POST /api/v1/auth/scim/group-mappings` and `{"group_id": "…", "org_id": "…", "role_id": "…"}` (`org_id` is the tenant id for the root unit). You must be able to give that role at that unit yourself. Current members get the role at once.
3. From then on, people added to or removed from the group gain or lose the role at once. A role someone was given by hand is never removed by SCIM, and a role two groups give stays while the person is in either.

Remove a rule with `DELETE /api/v1/auth/scim/group-mappings/{id}`; deleting the group in the provider removes its rules and the roles it gave.

## 5. What is supported

- Filters: `eq`, `ne`, `co`, `sw`, `ew`, `pr`, joined with `and` / `or` and parentheses. Users by `userName`, `externalId`, `emails`, `active`, `id`. Anything else answers `400` with `scimType` `invalidFilter`.
- `PATCH` (RFC 7644 §3.5.2): `add`, `replace`, `remove`, with paths such as `active`, `name.givenName`, `emails[type eq "work"].value`, or without a path. Attributes Autoxide does not keep (enterprise extension, phone numbers, title…) are ignored.
- Paging: `startIndex` (from 1) and `count` (at most 200).
- Not supported: bulk, sorting, ETags, password change.

Errors: `401` without a valid SCIM token, `403` when the plan lacks `auth.scim`, `404` for an id of another tenant, `409` `uniqueness` when `userName`, `externalId` or the email belongs to another account, `400` `invalidValue` / `invalidSyntax` / `invalidPath` / `noTarget` / `mutability` for a wrong body.

## Personal data

The tenant is the controller of its staff data and Autoxide processes it on the tenant's instructions (Vietnam's Personal Data Protection Law 91/2025/QH15 — check the original text with a legal expert). Only the attributes listed above are stored; the SCIM link is exported with the account and deleted with it.
