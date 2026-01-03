## 2024-01-01 - [Input Validation for External IDs]
**Vulnerability:** User input (`location_id`) was directly used in URL construction (`format!`) without validation, leading to potential parameter injection or malformed requests.
**Learning:** Even simple "IDs" can be vectors for injection if not strictly validated. Relying on string formatting for URLs is risky.
**Prevention:**
1.  **Strict Allowlist Validation:** Only allow expected characters (e.g., alphanumeric).
2.  **Safe URL Construction:** Use URL builder methods (like `reqwest::RequestBuilder::query`) that automatically handle encoding.

## 2025-05-18 - [DoS in Callback Handler]
**Vulnerability:** The `callback_query_handler` split the callback data string by `:` and accessed `parts[1]` without checking the length for certain actions ("edit", "delloc"). A malicious or malformed callback data (e.g., just "edit") would cause the bot to panic and crash (Denial of Service).
**Learning:** `split` operations on user input must always be followed by length checks before accessing indices. Assumptions about input format are dangerous.
**Prevention:**
1.  **Always Check Length:** Before accessing `parts[n]`, ensure `parts.len() > n`.
2.  **Pattern Matching:** Use pattern matching on slices (e.g., `match parts.as_slice() { [action, id, ..] => ... }`) which is safer and more idiomatic in Rust.
