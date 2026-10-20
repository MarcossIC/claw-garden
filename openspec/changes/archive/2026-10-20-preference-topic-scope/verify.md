# preference-topic-scope — Verificación

## Linux (2026-10-20)

Sobre `ae60565`, en `target`, con el compilador de `rust-toolchain.toml`:

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` sale con 0.
- Cada commit de código de las [tareas](tasks.md) (`c3409b7`, `545edff`) pasó la suite y clippy antes de commitearse.
- `the_agent_contract_matches_its_file` pasa sin bendecir: la línea nueva de `protocol.md` está fuera de la apertura que se manda como instrucciones, y ningún schema ni descripción de herramienta cambió.

## El binario real

Con `target/debug/clawg` de un build de tests, `CLAWG_TEST_HOME` en un directorio temporal, `CLAWG_TEST_EMBEDDER=fake` y `CLAWG_TEST_DEFAULT_PORT=off`, un daemon `dev` en `--port 0`:

- una `user_preference` guardada con `project_id` `pa` y `topic_key` `style/indent` vuelve en `POST /v1/search` desde `pb` con `"topic_key":"style/indent","project_id":"pa"`;
- `GET /v1/memories/memory/1/save?agent=a` responde con `"project_id":"pa"`;
- `clawg stop` lo para.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| R1 | `a_search_hit_and_the_save_read_carry_the_project` falló por la aserción: el hit no traía `project_id` |
| R2 | Los tests de `tools.rs`, `client.rs` y `server.rs` no compilaban sin las firmas nuevas y el campo. Con la regla anterior (`revised_from` devolviendo siempre verdadero) fallan por la aserción `a_hit_of_another_project_does_not_show_its_topic_key`, `a_hit_without_a_project_shows_its_key_only_to_a_bridge_without_one`, `format_save_shows_the_key_only_where_a_save_revises_it`, `search_memory_shows_a_key_only_in_the_project_it_revises` y `read_memory_shows_a_key_only_in_the_project_it_revises`; `a_hit_from_an_older_daemon_shows_its_key` pasa en los dos casos, porque fija lo que no cambia |
| R3 | `the_protocol_says_where_a_key_is_shown` falló antes de cambiar `protocol.md` |

## Lo que no se midió

- Cuántas veces un agente de verdad guarda bajo una clave de otro proyecto, o inventa la misma clave sin verla: haría falta un perfil de datos con varios proyectos y un cliente real. Si se mide que pasa, C de la [exploración](exploration.md) se suma después.
- Ningún ranking ni recuperación cambia (los hits, su orden y su puntaje son los mismos; sólo el texto de ` topic=K` del puente), así que no hay `rag-eval`. El golden set tiene un solo proyecto y `project_id` `None`, y la vista del agente pasa `None` como proyecto del save, así que su texto no cambia.
- Windows no se corrió a mano: el cambio no toca código que dependa de la plataforma, y el CI del pull request cubre la suite y clippy allí.
