<!-- SPDX-License-Identifier: MIT
  Copyright (c) 2026 WinTapNetAdapterCx contributors -->

# SPDX header policy

This repository uses the MIT SPDX identifier:

`SPDX-License-Identifier: MIT`

Governed files use comment syntax appropriate to their format:

| Files | Header |
| --- | --- |
| Rust, C, C++, and headers | `// SPDX-License-Identifier: MIT` |
| PowerShell, shell, YAML, TOML, CMake, and text metadata | `# SPDX-License-Identifier: MIT` |
| INF files | `; SPDX-License-Identifier: MIT` |
| Markdown | `<!-- SPDX-License-Identifier: MIT` followed by the copyright line and `-->` |

Shebangs remain first. YAML front matter remains intact, with the Markdown
header immediately after its closing delimiter. Strict JSON (`CMakePresets.json`),
`LICENSE`, binary files, and generated outputs are explicit exclusions because
comments would be invalid or the file is not source-controlled text policy
input. The validator reports exclusions in full-tree mode.

Install the repository hook once with:

```powershell
git config core.hooksPath hooks
```

The hook validates staged content before commit creation. CI runs the same
policy in full-tree mode as the required `SPDX headers` check; a local hook
bypass does not bypass pull-request or protected-branch enforcement.
