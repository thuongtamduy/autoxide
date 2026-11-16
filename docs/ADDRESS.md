# Administrative units and addresses

[Tiếng Việt](ADDRESS.vi.md)

The `address` module keeps the official list of administrative units, shared by every tenant —
for Vietnam since 2025-07-01: provinces and centrally governed cities (level 1) and communes, wards
and special zones (level 2), without districts — and checks addresses for other modules (the
public profile of a store, later the shipping addresses of customers).

## Read it

No sign-in needed; answers carry an `ETag` that changes when a new list is imported and
`Cache-Control: public, max-age=86400`.

| Request | Answer |
|---|---|
| `GET /api/v1/address/provinces?country=VN&as_of=YYYY-MM-DD` | provinces in force that day (default: today), in code order |
| `GET /api/v1/address/provinces/{code}/communes` | communes of a province |
| `GET /api/v1/address/units?q=ha noi&province=01&level=2` | search by name, with or without accents |
| `GET /api/v1/address/units/{code}` | one unit and the unit above it |
| `POST /api/v1/address/validate` | `{country, province_code, commune_code, line}` → the address with names, or 422 |

MCP tools: `address.list_provinces`, `address.list_communes`, `address.search_units`,
`address.validate_address`.

## Import the list

The server ships with the Vietnamese list in force from 2025-07-01 (34 provinces and cities, 3,321
communes, wards and special zones, per decision QĐ 19/2025/QĐ-TTg; taken from
provinces.open-api.vn v2, which republishes the statistics office's list — check it against the
original decision if you need legal certainty). Import it once per installation:

```bash
autoxide task address.import
```

`autoxide doctor` warns while nothing is imported, and when a newer list ships with an update.

To import another list (a later change by the state), get it from the official source — the decision
**QĐ 19/2025/QĐ-TTg** (list and codes of administrative units, in force from 2025-07-01) and the
electronic list of the statistics office ("Danh mục hành chính Việt Nam") — never from an
unofficial copy, and never generated. Put two files in a directory:

`manifest.json`:

```json
{
  "country": "VN",
  "title": "QĐ 19/2025/QĐ-TTg",
  "source_url": "<address of the official file you downloaded>",
  "retrieved_on": "YYYY-MM-DD",
  "effective_from": "2025-07-01"
}
```

`units.csv` (UTF-8, comma-separated, first line exactly as shown; codes keep their leading zeros):

```text
level,code,name,kind,parent_code
1,01,<province name>,city,
2,00004,<commune name>,ward,01
```

`kind` is `province`, `city`, `commune`, `ward` or `special_zone`; a commune's `parent_code` is the
code of its province. Then:

```bash
autoxide task address.import path=/path/to/the/directory
```

Importing the same file again changes nothing. When the state changes the list, import the new
one: units that disappeared or changed name, kind or province are closed on the new date and the
new ones opened — addresses written before keep the names they were written with.
`autoxide doctor` shows the list in use.
