# secret-coverage — Verificación

## Linux

Sobre el último commit del cambio:

- `cargo test --workspace`: pasa entero, `tests/binary.rs` incluido.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sin advertencias.
- `the_agent_contract_matches_its_file` no se movió: el cambio no toca lo que ve el modelo.

Cada test de la [spec](spec.md) falló antes de su código: por no compilar (los tipos nuevos, `Scanned`, `Purged.pieces`, `Purged.revived`, `Purged.workspaces`, `mask_reasons`) o por la aserción (`a_verdict_reason_is_stored_masked`, `a_failure_is_stored_masked`, `purge_keeps_other_agents_and_longer_saves_in_the_outbox`).

## Falsos positivos: el corpus del repositorio

Cada archivo versionado (`git ls-files`: código, docs, `openspec/`, workflows y manifiestos) pasado por `find_secrets` con las reglas nuevas, con un ejemplo temporal que imprimía sólo los matches de los tipos nuevos o ampliados (`credential`, `huggingface_token`, `slack_webhook`, `slack_token`, `stripe_key`, `private_key`). Los de `private_key` son las cabeceras RSA y OpenSSH de los tests y las specs, que ya se enmascaraban antes del cambio. Los nuevos son seis, todos de `credential`, y ninguno en prosa:

| Archivo | Valor |
|---|---|
| `crates/clawg-cli/src/client.rs` | `let token = "0123456789abcdef…"`, un token de test |
| `crates/clawg-cli/tests/api.rs` | `let token = "test-token-0123456789abcdef"`, un token de test |
| `crates/clawg-core/src/memory/redact.rs` (dos) | los casos de la tabla de `credential` |
| `openspec/changes/secret-coverage/spec.md` (dos) | los ejemplos de la spec |

Todos son literales con forma de secreto, que es lo que la regla tiene que tomar. Ningún `token: string`, `api_key: Option<String>`, `max_tokens`, `secret_name` o `${{ secrets.X }}` de los workflows salió. `hf_` no tomó ningún nombre de clase ni `hf_hub_download`. El ejemplo se borró después de la corrida.

## Un daemon real, perfil dev, home de prueba

`target/debug/clawg` del build de tests, con `CLAWG_TEST_HOME` en un directorio temporal y `CLAWG_TEST_EMBEDDER=fake`. Nunca se tocó `~/.clawg*`.

| Paso | Resultado |
|---|---|
| `POST /v1/memories` con `AWS_SECRET_ACCESS_KEY=<40>`, `OPENROUTER_API_KEY=sk-or-v1-<hex>` y `api_key: Option<String>` | `201`, `redacted: ["credential"]`; `memories list` muestra `AWS_SECRET_ACCESS_KEY=[REDACTED:credential]` y el `Option<String>` intacto |
| Dos saves bajo `deploy/target`, el segundo con `secret = zebra-quokka-7741` | el segundo reemplaza al primero (`replaced: [#2]`) y se guarda con `secret = [REDACTED:credential]` |
| `clawg secrets` con el daemon andando | `no stored memory holds a secret of a known format` |
| `clawg secrets --mask-reasons --yes` con el daemon andando | se niega: `…run \`clawg stop\` first, then mask the reasons`, código 1 |
| `clawg stop`, `clawg purge 3 --yes` | `purged memory #3`, `back as the current version of its topic: #2`, código 0 |
| Una fila escrita directo con `HF_TOKEN=hf_<34>`, un `reason` con `ghp_<36>` y un `workspace.md` con la fila en AUTO | `clawg secrets` lista `memory 4  huggingface_token  live` y `pair 2 1  github_token`, con las dos líneas de qué hacer |
| `clawg secrets --mask-reasons --yes` | `masked the reason of pair 2 1 (github_token)`; un segundo `clawg secrets` ya no lista el par |
| `clawg purge 4 --yes` | nombra el `workspace.md` (`AUTO section, rewritten by the agent's next recall_context in that project`) sin cambiarlo; `clawg secrets` queda vacío |
| `clawg secrets --yes` | error de uso de clap: `--yes` pide `--mask-reasons` |

## `last_error`

Leído el código, ningún error del worker cita el contenido del save: el evento del outbox lleva el contenido vacío y los errores de ONNX Runtime, del tokenizer y de SQLite no repiten su entrada. Un error de serde sí repite valores del evento (`invalid type: string "…"`), y `a_failure_is_stored_masked` lo provoca con un `importance` que es un token: con el cambio, el `last_error` guardado no lo tiene. No se provocó un fallo real del embedder.

## `rag-eval`

No aplica: el cambio no toca el ranking ni la búsqueda. El texto enmascarado de un save nuevo cambia el vector de ese save, como con cualquier regla del detector, y el golden set no tiene secretos.

## Lo que no se verificó

- **Windows.** La suite corre en el CI del PR.
- **Un perfil real.** `clawg secrets` sobre el perfil dev con el binario nuevo es la medición sobre datos reales: cada fila nueva que lista es un secreto que se había guardado en claro o un falso positivo que hay que mirar. Necesita autorización y queda para quien lo instale.

## Veredicto

Sin CRITICAL. Windows queda como WARNING hasta el CI del PR, y el perfil real como SUGGESTION.
