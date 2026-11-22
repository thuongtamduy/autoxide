- `content`: shared sections — blocks placed on many pages with `content.shared_section` and edited in one place
  (`/api/v1/content/shared-sections`). Public pages show the section's current blocks without being published
  again; deleting a section still in use answers `409 content.shared_section_in_use`.
