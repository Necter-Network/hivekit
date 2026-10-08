# Project templates

Minimal starting points, one per language. `necter-init <language> <dir>` (installed by
`install.sh`) copies one of these and replaces `__NAME__` with the directory name.

| Template | Build | Run locally |
|---|---|---|
| `rust` | `hivec build` | `hivec run dist/<name>.hbc addNumbers '{"a":2,"b":3}'` |
| `go` | `go mod tidy && hivec build .` | `hivec run dist/<name>.hbc addNumbers '{"a":2,"b":3}'` |
| `typescript` | `npm install && npx hivec build counter.ts` | `npx hivec run dist/counter.hbc increment 5` |
| `javascript` | `npm install && npx hivec build counter.js` | `npx hivec run dist/counter.hbc increment '{"by":2}'` |
| `python` | `pip install -r requirements.txt && hivec build counter.py` | `hivec run dist/counter.hbc increment '{"by":2}'` |
