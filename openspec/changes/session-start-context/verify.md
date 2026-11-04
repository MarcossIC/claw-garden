# session-start-context — Verificación

## Suite y clippy

En Linux x86_64, build debug, toolchain 1.99.1, en `target`, sobre `403823a`: el último commit de código y de `docs/reference`. Los commits que siguen sólo tocan esta carpeta, el mapa de openspec y el ROADMAP.

- `cargo test --workspace --no-fail-fast` pasa. La base (`592a6e0`, antes del primer commit) también pasaba entera, con los mismos ignorados. Lo nuevo son los tests de la [spec](spec.md).
- `cargo clippy --workspace --all-targets -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy -p clawg-cli -- -D warnings`, lo que construye el release, también.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin bendecir: lo que el modelo ve del servidor MCP no cambió.
- Windows queda para el CI del pull request. El cambio no toca código `cfg(windows)`: `prompt_builder.rs`, `workspace.rs` y `synthesis.rs` son los mismos en las dos plataformas, salvo las rutas de los archivos, que no cambian.

## Los tests en rojo

El de SC6 se corrió antes del arreglo. Los del recorte (SC2 a SC5) se corrieron con lo que necesitan para compilar ya puesto: `PromptWorkspace`, el informe de la regeneración y `PromptBlock::Workspace`. Faltaba el recorte en `fit_to_budget`. En ese estado los 108 tests que ya tenía `prompt_builder.rs` pasaban sin cambios: el prompt armado con la estructura nueva era byte a byte el de antes.

| Requisito | Test | Cómo fallaba |
|---|---|---|
| SC1 | `the_regeneration_reports_each_line_it_wrote` | Se escribió antes del informe: `EmittedIds` no tenía `rows`, así que no compilaba |
| SC2 | `a_stocked_workspace_leaves_room_for_the_last_session` | El arranque con 18 preferencias y un resumen medía 14.484 bytes con un presupuesto de 8.000: el resumen quedaba afuera y el workspace salía entero |
| SC2 | `the_auto_rows_over_half_go_before_the_hits` | Se cortaban los dos hits y quedaban las cinco entradas de AUTO |
| SC3 | `with_nothing_waiting_the_auto_rows_take_the_whole_budget` | 14.638 bytes con un presupuesto de 8.000: ninguna entrada se cortaba |
| SC4 | `the_auto_rows_go_by_importance_then_newest` | Quedaban las cuatro entradas |
| SC4 | `the_fit_names_each_block_it_cut` | El recorte nombraba `recent`, no `workspace` |
| SC5 | `what_the_user_wrote_is_never_cut` | Las tres entradas de AUTO seguían en el prompt |
| SC5 | `a_workspace_over_the_budget_is_kept_and_said`, `the_notes_of_the_prompt_follow_its_language` | La frase de antes: `The workspace alone is over the budget…` |
| SC6 | `the_reminder_leaves_out_what_an_import_wrote` | El aviso daba 28, no 3 |

Después del arreglo pasan todos.

## La spec contra el código

| Requisito | Dónde |
|---|---|
| SC1 | `EmittedRow` (`crates/clawg-core/src/memory/workspace.rs:320`), armado en `render_auto_section` (`:517`) con la línea de `memory_line` (`:579`) o de `format_skill_line`, las mismas que usa `render_rows`; `fetch_auto_rows` lee `importance` y `created_at` (`map_memory`, `:495`) con las mismas condiciones |
| SC2 | `AUTO_ROOM_DIVISOR` (`crates/clawg-core/src/memory/prompt_builder.rs:74`); el paso primero de `fit_to_budget` (`:482`, `waiting` en `:497`); las dos rutas pasan por `assemble` |
| SC3 | el último paso de `fit_to_budget`, antes de decir que el texto sigue pasado (`:523`) |
| SC4 | `PromptWorkspace::next_to_cut` (`:1197`), `cut_auto` (`:556`), `PromptWorkspace::rows` en `Cut.printed`, `PromptBlock::Workspace` (`:157`); el daemon registra `context.cut` con `PromptBlock::as_str`, sin cambios |
| SC5 | `PromptWorkspace` (`:1150`) guarda la parte de la persona aparte, y sólo una `AutoEntry` con fila se corta; `workspace_for_prompt` (`:1225`) reconoce cada entrada por su texto; la frase en `format_cut` (`:735`) |
| SC6 | `unreviewed_condition` (`crates/clawg-core/src/memory/synthesis.rs:44`) en `unreviewed_count` (`:302`); `synthesis_core` sigue con `PENDING` |

## Lo que no se midió

- **Un store real.** El arranque con las 18 preferencias que midió la exploración se probó con un store de la misma forma armado en el test. Falta un `session-start` con Claude Code sobre el perfil `dev` y un store con preferencias e import, y eso necesita una máquina con el cliente instalado.
- **`rag-eval --diff`.** No aplica: el ranking no cambia y el golden no tiene workspace.
- **Windows,** que queda para el CI del pull request.

## Lo que queda

- El CI del pull request en `windows-latest`.
- Archivar y subir la versión, fuera de este cambio.
