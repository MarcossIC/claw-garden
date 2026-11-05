# session-start-context — Verificación

## Suite y clippy

En Linux x86_64, build debug, toolchain 1.99.1, en `target`, sobre `403823a`: el último commit de código y de `docs/reference`. Los commits que siguen sólo tocan esta carpeta, el mapa de openspec y el ROADMAP.

- `cargo test --workspace --no-fail-fast` pasa. La base (`592a6e0`, antes del primer commit) también pasaba entera, con los mismos ignorados. Lo nuevo son los tests de la [spec](spec.md).
- `cargo clippy --workspace --all-targets -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy -p clawg-cli -- -D warnings`, lo que construye el release, también.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin bendecir: lo que el modelo ve del servidor MCP no cambió.
- En Windows, el job `test (windows-latest)` del CI de #129 pasó sobre `26c52fb`, la punta del pull request, que contiene `403823a`. El cambio no toca código `cfg(windows)`: `prompt_builder.rs`, `workspace.rs` y `synthesis.rs` son los mismos en las dos plataformas, salvo las rutas de los archivos, que no cambian.

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

- **El arranque real.** El arranque con las 18 preferencias que midió la exploración se probó con un store de la misma forma armado en el test. Falta un `session-start` real con Claude Code: está en [«Pendiente en prod»](#pendiente-en-prod). El contexto con consulta sí se midió en un store real ([abajo](#un-store-real-2026-11-04)).
- **`rag-eval --diff`.** No aplica: el ranking no cambia y el golden no tiene workspace.

## Un store real (2026-11-04)

`recall_context` con un presupuesto de 8.000 bytes, con clawg 0.76.0 publicado, el perfil `prod` y un store real con la sección AUTO llena:

- El bloque `BUDGET` dice `Recortado para entrar en 8000 bytes; el contexto completo mide 20728 bytes.` y nombra 19 ids en «Quedaron afuera».
- LAST SESSION sale entero, con hits de RECENT y de LONG-TERM debajo. La propuesta midió el 2026-11-03 un `recall_context` de 12.000 bytes que devolvía sólo el workspace.
- El aviso de ese contexto dice 1.612 memorias sin revisar. No se comparó con un recuento con lo importado adentro: lo que prueba SC6 es `the_reminder_leaves_out_what_an_import_wrote`.

Esto recorre `build_memory_prompt`, no `build_session_prompt`, que es la ruta que usa el arranque y que sólo se probó con el store del test.

## Pendiente en prod

Con clawg 0.76.0 o posterior publicado, y los hooks instalados a mano en `.claude/settings.local.json` de un repositorio con preferencias, historia importada y un `session_summary` guardado, como pide el piloto del [ROADMAP](../../../ROADMAP.md):

1. **Que el arranque traiga el traspaso:** en una sesión nueva, el contexto que inyecta `session-start` trae LAST SESSION entero y, si algo no entró, el bloque `BUDGET` nombra las filas cortadas.
2. **Que la corrida quede a la vista:** `clawg doctor` muestra que esa corrida inyectó contexto.

## Al archivar (2026-11-04)

- **El código sigue diciendo lo mismo.** `git diff 403823a 4f5f0e5 -- crates/clawg-core/src/memory/prompt_builder.rs crates/clawg-core/src/memory/workspace.rs crates/clawg-core/src/memory/synthesis.rs` sale vacío, y cada test que nombra la spec sigue en su archivo. Este registro no repite la suite porque archivar no cambia código.
- **El código ya salió.** `git tag --contains 403823a` da `v0.76.0`, publicado con la CI de `a4e1ae8` en verde.
- **SC1 a SC6 en las specs.** SC1 a SC5 pasan a [`specs/retrieval`](../../../specs/retrieval/spec.md), SC6 a [`specs/synthesis`](../../../specs/synthesis/spec.md) (R8 y «Memoria sin revisar») y el bloque `workspace` a [`specs/metrics`](../../../specs/metrics/spec.md) (R14), como reglas de ahora, sin los «hoy falla» ni los «como hoy». Se agregan al consolidar tres cosas que el código ya hace y la spec callaba: la parte de la persona es también lo que hay entre `AUTO-END` y `USER-START`; una fila cortada sigue en `shown`, así que sus conflictos siguen en el bloque; y si la regeneración falla ninguna entrada es conocida. Las citas de línea de los apartados que se reescribieron son las de `4f5f0e5`; las demás de `prompt_builder.rs` en `specs/retrieval` conservan los números de antes de este cambio.
- **La frase de SC5 puede decir de más (WARNING).** Si la regeneración falla, el informe queda vacío (`crates/clawg-cli/src/daemon/api.rs:1583-1586`) y toda la sección AUTO se queda. Si con ella el texto pasa el presupuesto, `format_cut` dice que lo que escribió la persona lo pasa sola, aunque lo que sobra lo escribió la regeneración. La spec lo dice tal cual y ninguno de los tests que nombra fija este caso. Sólo afecta al texto de la nota en ese camino de falla, y archivar no cambia código.
- **Sin revisión en GitHub.** #129 se mergeó sin reviews: CodeRabbit la saltó porque la base no es la rama por defecto.
- **La versión sube a 0.77.0.** 0.76.0 ya tiene tag, así que no aplica la excepción de embedder-footprint, la de una versión sin publicar que ya trae el código. Subirla en `develop` no publica nada: publica el merge a `main`.

Sin CRITICAL en Linux ni en el CI de Windows. «Pendiente en prod» y la frase de SC5 quedan como WARNING.
