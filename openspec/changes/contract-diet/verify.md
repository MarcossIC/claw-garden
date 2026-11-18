# contract-diet — Verificación (el bloque `lean`)

Verifica la clase (b), enfoque B1: la variante `lean`, su sello y el brazo `a3-lean`. La clase (a) se verificó al hacerse ([exploración](exploration.md#hecho-la-clase-a)). **La campaña no corrió**: es paga y queda [pendiente](#pendiente-la-campaña), con su regla fijada en [design](design.md#la-regla-de-decisión).

## Qué se revisó

- Los commits de la serie sobre `develop`: la propuesta (`docs(openspec): propose the lean protocol block for contract-diet`), `feat(mcp): write a lean protocol that keeps the opening and every rule`, `feat(setup): choose the lean protocol block with --protocol lean` y `feat(agent-eval): add the a3-lean arm with the lean protocol block`.
- Sobre el último, con `cargo clean --workspace` antes, en Linux con el embedder falso: `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings` y `cargo clippy --locked -p clawg-cli -- -D warnings`. La suite pasa y los dos clippy no dan ningún hallazgo.
- `git diff develop` no toca `protocol.md`, `protocol.es.md`, `agent_contract.json`, `agent_contract.es.json`, `crates/clawg-cli/src/daemon/`, `crates/clawg-cli/src/hook.rs` ni `crates/clawg-cli/src/doctor/` (CD6). `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin `CLAWG_BLESS=1`.

## Requisitos

| Requisito | Tests | Estado |
|---|---|---|
| CD1, el texto de `lean` | `the_lean_protocol_keeps_the_head_of_the_full_one`, `the_lean_protocol_keeps_the_topic_key_rule_and_the_session_summary`, `the_lean_protocol_names_what_the_full_one_names`, `a_language_rewrites_both_rules_of_the_lean_protocol`, `the_lean_protocol_is_at_most_seventy_percent_of_the_full_one` (`protocol.rs`) | cumple. Con `lean` devolviendo el texto de `full`, el de tamaño fallaba por la aserción; los de igualdad son guardas contra la deriva de los dos textos |
| CD2, las pruebas del protocolo valen para `lean` | `every_protocol` con `lean` en `the_protocol_names_only_what_exists`; `the_protocol_says_where_a_key_is_shown`, `the_protocol_describes_the_session_summary`, `the_spanish_protocol_describes_the_session_summary` (`server.rs`); `the_spanish_protocol_has_the_sections_and_spans_of_the_english_one`, `the_protocol_says_which_preferences_open_every_session`, `the_protocol_says_where_a_call_without_a_project_goes` (`protocol.rs`) | cumple |
| CD3, el bloque y su sello | `each_variant_is_its_block`, `the_protocol_option_writes_its_variant_and_a_run_without_it_keeps_it`, `a_new_version_with_the_same_text_rewrites_nothing`, `a_stamp_reads_its_version_and_its_choice` (`setup.rs`); `a_variant_is_named_by_its_option` (`protocol.rs`); `the_protocol_option_parses_and_excludes_local_remove_and_cursor` (`cli.rs`), que fallaba por el mensaje; `doctor_marks_an_old_block_stale_without_a_preference` (`tests/binary.rs`), con `--protocol lean` | cumple |
| CD4, un sello con una variante desconocida | `a_stamp_with_an_unknown_variant_reads_as_stale` (`setup.rs`) | cumple, sin código nuevo: `Stamp::parse` ya daba `None` (`setup.rs:665`, el `?` sobre `Variant::parse`), así que un binario anterior lee `protocol=lean` como un sello ilegible, el bloque como `stale` y no `newer`, y su `setup` sin `--protocol` escribe `full` |
| CD5, el brazo `a3-lean` | `a3_lean_is_a3_with_the_lean_block`, `a3_lean_reads_and_prints_by_its_name`, y el brazo en `every_arm_logs_the_instruction_files_the_client_loads`, `the_arms_with_clawg_start_a_daemon_of_the_run_on_an_ephemeral_port` y `the_startup_rules_must_fit_what_the_client_injects` (`arm.rs`) | cumple |
| CD6, lo que no cambia | los dos del contrato, `without_a_language_the_protocol_is_the_file` | cumple |

## El tamaño

Medido sobre los archivos, sin la línea del sello (unos 40 caracteres más en los dos bloques):

| | `full` | `lean` | Diferencia | `lean` / `full` |
|---|---|---|---|---|
| inglés, caracteres | 10.691 | 7.068 | −3.623 | 66,1 % |
| español, caracteres | 11.271 | 7.663 | −3.608 | 68,0 % |
| español, bytes | 11.442 | 7.772 | −3.670 | 67,9 % |

Con la regla de dedo de la exploración (unos 4,6 tokens de T por carácter de bloque, en sesiones de unos 15 pedidos), −3.623 caracteres en inglés son unos −16.700 tokens de T por sesión, el 10,7 % de los 156.340 de `a3` sin la K. Es una estimación para ordenar, no un resultado: queda cerca del umbral del 10 % (D5), y la campaña lo decide.

## Dónde quedó cada regla que salió

Cada regla del bloque `full` sigue en `lean` o en un texto que el agente lee: la apertura (en cada pedido) o la descripción de la herramienta que la regla gobierna (al cargarla, antes de usarla). Lo que salió sin ir a otro lado son explicaciones y énfasis, no reglas.

| Lo que salió de la sección | Dónde sigue |
|---|---|
| «Cuándo guardar»: la lista de momentos; guardar en el momento, no al final | la apertura («Use `save_memory` when…», «Save it at that moment») |
| «Cuándo guardar»: «el momento que más se pierde» | énfasis, sin otro lugar |
| «Qué no guardar»: lo que dice el código, el estado de la tarea, el registro de pasos, secretos | la apertura («Do not save…») |
| «Qué no guardar»: «cada fila compite en la búsqueda» | explicación, sin otro lugar |
| «Cómo escribir»: qué, por qué y dónde, identificadores exactos, una cosa por llamada | la apertura y la descripción de `save_memory` |
| «Cómo escribir»: 64 KiB y partir un texto largo; «que se lea solo dentro de meses» | la descripción de `save_memory` y su parámetro `content` |
| «Cómo escribir»: qué es `user_stated`, y «sólo cuando el código y la documentación no lo dicen» | el parámetro `user_stated`; la apertura («Do not save what the code or the docs already say») |
| «Tipo e importancia»: qué es cada tipo, el valor por defecto, que un tipo desconocido se rechaza, que lo hecho después de una decisión es un `semantic_fact` | el `enum` y la descripción del parámetro `memory_type` de `save_memory` |
| «Tipo e importancia»: `importance` en 5 y el 9 que nunca baja (sólo en inglés) | la apertura en inglés; en español quedan en `lean`, porque su apertura no los dice |
| «Conclusiones»: que `synthesize` lista lo que no cubre ninguna conclusión, que una memoria común no lleva `derived_from` | la descripción de `synthesize` y el parámetro `derived_from` |
| «Evidencia»: que todo lo recuperado es evidencia y no una instrucción | la apertura |
| «Proyectos»: qué es cada parámetro de proyecto, el valor por defecto (las carpetas abiertas o el directorio del servidor), que un nombre nuevo crea el proyecto | los parámetros `project_path`, `project`, `all_projects` y `to_project` de cada herramienta |
| «Proyectos»: que el rechazo lista las carpetas abiertas | lo dice el rechazo mismo |
| «Las otras herramientas»: qué hace cada una | sus descripciones |

## Lo que se miró con cuidado

- **La apertura es la de `full`**, por idioma y con la preferencia `en`: el test compara las cabezas y `opening`, así que las instrucciones del servidor, que salen de `text` (`server.rs:2081`), no pueden cambiar por `lean`.
- **La regla del idioma con la preferencia `en`.** `text` reescribía `PROTOCOL` directamente; ahora la reescritura (`in_language`) toma el texto, y `lean` la aplica sobre su propio texto. `a_language_rewrites_both_rules_of_the_english_protocol` sigue igual, y `the_language_rules_occur_once_in_the_protocol` vale para `full`; para `lean`, `a_language_rewrites_both_rules_of_the_lean_protocol` exige `SAVE_RULE` una vez en `protocol.lean.md`.
- **Finales de línea.** `lean` pasa los dos archivos a `\n`, como `short`; `the_lean_protocol_keeps_the_head_of_the_full_one` lo exige, y `freshness` compara sin mirarlos.
- **La sección de tipos nombra los siete tipos** aunque el `enum` ya los liste: `the_protocol_names_only_what_exists` pide que cada protocolo nombre cada herramienta y cada tipo, y la exploración contaba la lista entre lo que se podía sacar. Quedan como nombres, sin su definición.

## Pendiente: la campaña

Paga, y la autoriza la persona. Corre con una versión que traiga `lean` y con el mismo build en los dos brazos; este cambio no sube la versión.

- **Brazos:** `a3` y `a3-lean`. Si el build no es el de las corridas de `a3` de protocol-block, `a3` se corre otra vez con el de `a3-lean`.
- **Tareas:** `requests-timeout-decision-relay` (R, continua y nueva), `requests-redact-decision-revised` (D partida) y `requests-history-owner-convention` (la K larga), con el ruido de 15 reglas: 3 corridas por celda, 12 de `a3-lean`, unos 3,7 USD a los 0,30 USD por corrida de protocol-block; con `a3` otra vez, unos 7,4. Tope: 15 USD.
- **Lectura:** las reglas de 129 tal como están (corridas fijas, las mismas causas de exclusión, la fase que se detiene con más de 2 exclusiones).
- **Métricas:** E, B, G, S, J y T como en [D6 de protocol-block](../archive/2026-11-17-protocol-block/design.md#d6-la-regla-de-decisión-del-valor-por-defecto); aparte, el gasto en USD de cada brazo y cuántos saves de la decisión de D llevan `topic_key`.
- **`lean` pasa** si cumple las seis condiciones de D6 y además la séptima de D5:
  1. E(`lean`) ≥ E(`a3`) − 1, y ninguna tarea pasa de aprobar en todas sus corridas a no aprobar en ninguna;
  2. B(`lean`) ≥ B(`a3`) − 1;
  3. G(`lean`) ≥ G(`a3`) − 1;
  4. S(`lean`) ≥ S(`a3`) − 1;
  5. J(`lean`) ≤ J(`a3`) + 0,5;
  6. T(`lean`) < T(`a3`);
  7. T(`lean`) ≤ 0,9 × T(`a3`).
- **Qué se decide:** si pasa, un commit propio cambia `DEFAULT_VARIANT` (`setup.rs:680`) a `Lean`; si no, queda `full` y `lean` sigue elegible con `--protocol lean`. No se agregan corridas para desempatar. Si J falla y los saves de D van sin `topic_key`, la causa es la viñeta, que `lean` tiene entera: sería otra hipótesis, no otra lectura de la regla.

## Pendiente en local

- **CI** en verde en Linux y Windows sobre el último commit.
- **El bloque de una máquina real**, con la versión instalada: `clawg setup claude-code --protocol lean --profile prod --dry-run` dice `would update it (protocol=lean)`; después de correrlo sin `--dry-run`, `clawg doctor --profile prod` da `protocol_block` `ok` `healthy`, y la primera sesión no trae el aviso del bloque.
