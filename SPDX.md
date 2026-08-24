<!-- SPDX-License-Identifier: MIT
  Copyright (c) 2026 WinTapNetAdapterCx contributors -->

# SPDX header policy

This repository uses the MIT SPDX identifier:

`SPDX-License-Identifier: MIT`

Vendored WDK binding sources retain their declared dual license:
`SPDX-License-Identifier: MIT OR Apache-2.0`, together with the Microsoft
copyright and `License: MIT OR Apache-2.0` notice.

Governed files use comment syntax appropriate to their format:

| Files | Header |
| --- | --- |
| Rust, C, C++, and headers | `// SPDX-License-Identifier: MIT` |
| PowerShell, shell, YAML, TOML, CMake, and text metadata | `# SPDX-License-Identifier: MIT` |
| INF files | `; SPDX-License-Identifier: MIT` |
| Markdown | `<!-- SPDX-License-Identifier: MIT` followed by the copyright line and `-->` |

Shebangs remain first. YAML front matter remains intact, with the Markdown
header immediately after its closing delimiter. Strict JSON (`CMakePresets.json`), `LICENSE`, and generated `out/*` and
`target/*` outputs are explicit exclusions because comments would be invalid
or the files are generated/license text. These entries, including their
reasons, are maintained in `scripts/spdx-policy.psd1` and reported by the
validator in full-tree mode. Binary catalog, driver, executable, library, and
symbol files are excluded by their explicit extension entries in that same
manifest when tracked.

Install the repository hook once with:

```powershell
git config core.hooksPath hooks
```

The hook validates staged content before commit creation. CI runs the same
policy in full-tree mode as the required `SPDX headers` check; a local hook
bypass does not bypass pull-request or protected-branch enforcement. The hook
uses PowerShell 7 when available and falls back to Windows PowerShell 5.1.
