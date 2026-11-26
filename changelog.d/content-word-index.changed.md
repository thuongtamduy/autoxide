- **Breaking:** public content search (`GET /api/v1/content/public/posts?q=`, the word part of
  `GET /api/v1/content/public/search`, content suggestions) matches whole words, ignoring case and accents
  (`ca phe` finds "Cà phê"), and word prefixes from 3 letters (`lat` finds "Đà Lạt"). It no longer matches any
  substring (`alat` finds nothing). It covers the 1,000 newest matching entries.
