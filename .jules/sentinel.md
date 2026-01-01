## 2024-01-01 - [Input Validation for External IDs]
**Vulnerability:** User input (`location_id`) was directly used in URL construction (`format!`) without validation, leading to potential parameter injection or malformed requests.
**Learning:** Even simple "IDs" can be vectors for injection if not strictly validated. Relying on string formatting for URLs is risky.
**Prevention:**
1.  **Strict Allowlist Validation:** Only allow expected characters (e.g., alphanumeric).
2.  **Safe URL Construction:** Use URL builder methods (like `reqwest::RequestBuilder::query`) that automatically handle encoding.
