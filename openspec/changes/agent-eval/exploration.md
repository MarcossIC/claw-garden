# agent-eval — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (1, 5) | L (por fases; la primera es S y no gasta) | — (absorbe M3 de [revision-eval](../archive/2026-10-21-revision-eval/proposal.md), 48, archivado) | exploración (borrador) |

**Tesis:** `rag-eval` mide qué se recupera y `--agent-view` qué texto le llega al agente, pero nada mide si una sesión de un agente de código con clawg resuelve una tarea mejor, más rápido o más barato que sin clawg, ni cuánto daña una memoria vieja, ni qué calidad tienen los saves que escribe el agente. Cada cambio que tocó lo que ve el agente se verificó con una sesión manual de Claude Code (25, 41, 45). Este cambio propone convertir esa verificación en una medición repetible con sesiones reales de `claude -p`, empezando por un piloto chico y por la serie M3 que revision-eval dejó escrita y sin correr.

**De dónde sale.** De una investigación previa sobre `develop` en 0.27.0, que no está en el repositorio y que propuso dieciséis puntos (P1 a P16). Esta exploración los contrasta uno por uno contra `develop` en 0.68.0 (`0f09b2b`), donde muchas premisas cambiaron: 52, 44 y 48 se archivaron, `EvalIngest` ganó `save`, hay una preferencia de idioma, una herramienta más y un import que conserva fechas. Cada afirmación sobre el código cita `ruta:línea` de este árbol; lo que viene de la documentación de Claude Code dice qué página y qué se leyó.

## Premisas de la investigación, verificadas contra 0.68.0

| Premisa | Resultado |
|---|---|
| El contexto sin consulta de `session-start` lista sólo los saves propios de los últimos 7 días | **Cierta.** `RECENT_WINDOW_MS` (`crates/clawg-core/src/memory/prompt_builder.rs:56`, usado en `:322`). |
| El workspace trae preferencias y decisiones con `importance >= 8` en tier 1 | **Cierta** (`crates/clawg-core/src/memory/workspace.rs:346-411`). **Matiz que la investigación no vio:** una `architectural_decision` sube a tier 1 con `access_count >= 3`, no sólo con importancia ≥ 9 (`compute_tier`, `crates/clawg-core/src/memory/tier.rs:22-50`). Sembrar sin accesos deja fuera del workspace decisiones que en un store real estarían. |
| `EvalIngest` siembra todo «ahora», sin proyecto, sin clave y `user_stated` | **Desactualizada en parte.** `send` ya recibe `age_ms` (`crates/clawg-core/src/eval_api.rs:807-822`), y `save` (de 48) recibe proyecto y `topic_key` (`EvalSave`, `:675-682`; `:827-845`). Sigue siendo cierto que `save` siembra con edad 0 (`:843`), que todo sale `user_stated: true` y sin `derived_from` ni sesión (`explicit_event`, `:706-737`). |
| `POST /v1/memories` no acepta una fecha | **Cierta** (`SaveRequest`, `crates/clawg-cli/src/daemon/api.rs:387-410`). |
| No hay forma de sembrar un store fechado por el camino de producción | **Falsa desde memory-portability e history-import.** `clawg import --from clawg` lee el formato `clawg-memories`, cuya línea `memory` lleva `created_at`, `last_accessed_at`, `access_count`, `importance`, `tier`, `status`, `topic_key`, `created_by_type`, `source_session_id`, `superseded_by` y `disputed_against` (`crates/clawg-core/src/memory/portability/format.rs:92-117`), re-embebe con el modelo del daemon y no corre el detector (`openspec/specs/portability/spec.md:75-77`). No pide cambiar código. |
| `user-prompt-submit` sólo habla después de 30 minutos sin save | **Cierta** y sigue «Not measured yet» (`SAVE_REMINDER_AFTER`, `crates/clawg-cli/src/hook.rs:28-30`; `save_reminder`, `:142-163`). |
| Claude Code difiere las herramientas MCP detrás de `ToolSearch` | **Cierta, y más fuerte de lo que decía.** La investigación no pudo confirmar si había un umbral. La página de MCP (https://code.claude.com/docs/en/mcp, «Configure tool search», leída el 2026-10-31) dice que sin `ENABLE_TOOL_SEARCH` **todas** las herramientas MCP se difieren y que el umbral del 10 % del contexto sólo existe en el modo `auto`. Un servidor puede pedir que una herramienta cargue de entrada con `_meta["anthropic/alwaysLoad"]: true` (v2.1.285+), y quien configura el servidor puede pedirlo para el servidor entero con `alwaysLoad`. clawg no usa ninguno de los dos (ninguna coincidencia de `alwaysLoad` en `crates/`). |
| Son 12 herramientas | **Desactualizada:** son 13, con `search_project` (`crates/clawg-cli/src/mcp/agent_contract.json`). |
| La apertura del protocolo mide 1.948 caracteres y llega dos veces con `setup` | **Cierta la duplicación, cambió el tamaño.** Las instrucciones son `opening(&protocol::text(lang))` (`crates/clawg-cli/src/mcp/server.rs:1778`; `opening`, `crates/clawg-cli/src/mcp/protocol.rs:33-36`), y `setup` escribe el protocolo entero, apertura incluida (`block`, `crates/clawg-cli/src/setup.rs:559-565`). La apertura en inglés mide hoy 2.028 caracteres (`agent_contract.json`, campo `instructions`), a 20 del tope de 2.048 que fija `the_instructions_fit_older_clients` (`server.rs:2571-2577`). |
| `setup` respeta `CLAUDE_CONFIG_DIR` | **Cierta** (`crates/clawg-cli/src/setup.rs:625`, `:1087`). Registra el servidor con `claude mcp add --scope user` (`:122`), así que armar el brazo con hooks pide `claude` en el PATH del corredor. |
| `clawg-eval` no tiene biblioteca y `wilson` está duplicado | **Cierta, y ahora triplicado:** `rag-eval/stats.rs:35`, `conflict-eval/measure.rs:107` y `revision-eval/stats.rs:8`. **Y es una decisión tomada:** revision-eval decidió copiar lo compartido «con sus casos fijados» (`openspec/changes/archive/2026-10-21-revision-eval/design.md:18`), y el crate declara «No library target» (`crates/clawg-eval/Cargo.toml:11`). |
| El arnés de 48 queda por hacer y este cambio va «antes de 52 y 44» | **Desactualizada.** 48, 52 (untrusted-recall) y 44 (history-import) se archivaron y el import en prod ya se hizo. Lo que sí sigue vivo es M3 de 48: la propuesta dejó escritos sus escenarios, brazos, aislamiento y umbrales (`openspec/changes/archive/2026-10-21-revision-eval/proposal.md:23-55`), y su `verify.md` deja tres decisiones de producto pendientes de M3 (`verify.md:123-129`). |

Lo que la investigación no consideró y cambia el diseño:

- **La preferencia de idioma.** Con `es`, el protocolo pide guardar y buscar en español (`crates/clawg-cli/src/mcp/protocol.es.md:83-85`), y en inglés sin preferencia (`protocol.md:81`). Las instrucciones, las descripciones y el bloque de `setup` cambian con ella (`openspec/specs/setup/spec.md:113`). El idioma es un factor más de cada corrida.
- **LAST SESSION no tiene ventana de edad.** Todo contexto abre con el `session_summary` más nuevo del proyecto, de cualquier fecha (`openspec/specs/retrieval/spec.md:487-490`). Sembrar un resumen o no es una decisión de cada tarea, no un detalle.
- **El índice de docs del proyecto.** `search_project` indexa el fixture. Un trazador que aparezca en el índice invalida la tarea igual que si apareciera en un archivo.

## Veredictos

Impacto: qué tanto mejora la capacidad de decidir sobre el producto. Riesgo: qué puede salir mal al hacerlo o al leer sus números. Esfuerzo: Bajo, horas; Medio, 1 a 3 días; Alto, más.

| # | Propuesta | Veredicto | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|---|---|
| P1 | Diseño de brazos A0 a A5 | **Modificar** | Alto | Medio | Medio |
| P2 | Sembrar memorias viejas, con proyecto, clave y procedencia | **Modificar** (por `clawg import`, sin código) | Alto | Bajo | Bajo |
| P3 | `user-prompt-submit` mudo en sesiones de menos de 30 minutos | **Incorporar** (declararlo y medirlo con tareas reanudadas) | Medio | Bajo | Bajo |
| P4 | Herramientas diferidas detrás de `ToolSearch` | **Investigar** en el piloto, con la premisa ya confirmada | Medio-Alto | Medio | Bajo |
| P5 | Suite de tareas sobre un repositorio ficticio | **Incorporar**, en dos tandas | Alto | Medio | Alto |
| P6 | Corredor con `claude -p` aislado por corrida | **Incorporar** | Alto | Medio | Medio |
| P7 | Métricas y trazadores | **Incorporar** | Alto | Bajo | Medio |
| P8 | Apertura duplicada en `CLAUDE.md` | **Posponer** (decidir con el piloto) | Bajo | Bajo | Bajo |
| P9 | Número de corridas con el criterio de 33 | **Incorporar** | Alto | Bajo | Bajo |
| P10 | Costo estimado | **Modificar** (rehacer con el piloto y con precios verificados) | Medio | Medio | Bajo |
| P11 | Rúbrica de calidad de un save | **Modificar** (sólo lo determinista primero; el juez, después) | Medio-Alto | Medio | Medio |
| P12 | Benchmarks públicos | **Posponer** (todos) | Bajo ahora | Alto | Alto |
| P13 | Qué se automatiza en `clawg-eval` | **Modificar** (sin biblioteca nueva) | Alto | Medio | Alto |
| P14 | Encaje en openspec | **Modificar** (cambio propio que absorbe M3) | Alto | Bajo | — |
| P15 | Piloto mínimo | **Incorporar** (es el primer paso con gasto) | Alto | Bajo | Bajo |
| P16 | Telemetría OpenTelemetry de Claude Code | **Posponer** | Bajo | Bajo | Bajo |

Ninguna se descarta del todo. Dentro de P5, P11 y P12 se descartan partes, dichas en su punto.

## Por propuesta

Las dieciséis se agrupan en cinco bloques porque se implementan juntas: el arnés (P1, P5, P6, P7, P9, P13, P15), la siembra (P2), las decisiones de producto que el arnés destraba (P3, P4, P8), la calidad de los saves (P11) y lo externo (P10, P12, P16). P14 es el encaje de todo.

### Bloque 1. El arnés

**P1. Brazos — Modificar.** El problema es real: hoy no hay forma de separar «tener las herramientas», «tener conocimiento útil», «que llegue solo» y «que el agente lo busque». La separación en brazos es correcta, pero seis brazos por tarea multiplican el gasto y la mayoría de las preguntas se contestan con tres:

- **Núcleo: A0 (sin memoria), A3 (lo que instala `setup`) y A4 (memoria nativa de Claude Code con los mismos hechos).** A3 contra A0 es la hipótesis primaria y A3 contra A4 la pregunta de producto («qué agrega clawg sobre lo que el usuario ya tiene»).
- **A2 (sólo el puente, sin hooks ni protocolo en `CLAUDE.md`)** entra en la campaña pero no en el piloto: A3 − A2 es el efecto de los hooks, y con P3 ese efecto es en la práctica el de `session-start`.
- **A1 y A5** sólo como diagnóstico, como propone la investigación.
- **Brazo B de M3**, que ya está escrito: la sesión 2 sin `search_memory`, `read_memory` ni `list_memories` (`revision-eval/proposal.md:25`). Es un A3 recortado, no un brazo nuevo del corredor.
- **Un factor que falta: el idioma.** Se fija `preferences.json` por corrida (ausente, que es inglés, o `es`) y se reporta. La campaña núcleo corre en el idioma del usuario real; M3 ya pide los dos.

La paridad de conocimiento y de distractores entre brazos, el `CLAUDE_CONFIG_DIR` y el HOME nuevos por corrida, la memoria automática apagada fuera de A4 y el `--effort` explícito se mantienen como los describe la investigación: son lo que hace comparable la medición.

**Riesgo:** A4 con cientos de hechos no es el producto que el usuario usa (la memoria automática carga como mucho las primeras 200 líneas o 25 KB de `MEMORY.md`, según la investigación; no se volvió a verificar). Se reporta el tamaño de cada lado y no se concluye «A3 > A4» si A4 está truncado.

**P5. Suite de tareas — Incorporar, en dos tandas.** Hace falta un fixture que no sea clawg: `.claude/CLAUDE.md` ya contiene el conocimiento de este repositorio y A0 lo leería. El diseño de categorías (K1 a K5, S, C, N, Q, D, R) es bueno porque cada una se verifica sin juez. Cambios:

- **Primera tanda, para el piloto:** 6 tareas (K1, K2, K3, S, C, N), como dice P15. **Segunda tanda:** hasta unas 30 de conocimiento y 6 controles, más las de M3 (D con clave) y 4 a 6 R.
- **La categoría I (instrucción sembrada) se pospone.** Lo que medía lo cubre ya untrusted-recall (52), archivado, con el margen `  | ` y su test; repetirlo con sesiones pagas no cambia una decisión.
- **La auditoría de la suite suma el índice de docs**: un trazador no puede aparecer en el fixture ni en lo que `search_project` devuelve sobre él.
- El fixture y las tareas se versionan en `crates/clawg-eval/data/agent-tasks/`, como `conflict-pairs.json` y `revision-chains.json`.

**Riesgo:** es el trabajo grande y no se puede generar con un LLM sin contaminar la medición. Mitigación: el piloto descarta las tareas que A0 resuelve o que A5 no resuelve antes de escribir la segunda tanda.

**P6. Corredor `claude -p` — Incorporar.** Reusa lo que ya probaron las verificaciones 25, 41 y 45 (`--mcp-config` con `--strict-mcp-config`, `--settings` para los hooks, `stream-json` con `--include-hook-events`, un nombre de servidor único). La plantilla de la investigación sirve con tres ajustes: `clawg setup claude-code` necesita `claude` en el PATH porque registra el servidor con `claude mcp add` (`setup.rs:122`); el daemon de cada corrida se baja con `clawg stop` (regla de `.claude/CLAUDE.md`); y `preferences.json` se escribe en el HOME de la corrida antes de `setup`, porque el bloque sigue al idioma. `--bare` queda fuera de los brazos con hooks, como dice la investigación.

**P7. Métricas y trazadores — Incorporar.** Es la pieza de más valor y la más barata de mantener: todo sale de `stream.jsonl`, del diff y del store final, sin cambiar el producto. Los trazadores separan `never_reached` (problema de recuperación) de `retrieved_but_ignored` (problema de uso), que es la misma separación que `answer_reachable` y `answer_visible` de agent-view llevada al comportamiento. El daño neto atribuible, `success(A0) − success(brazo)` en S y C, es la única métrica que contesta «¿la memoria vieja engaña?» sin confundirla con «la tarea es difícil». Ajuste: `clawg metrics report` (`specs/metrics`) no tiene series por herramienta MCP ni por hook (`openspec/specs/metrics/spec.md:162-200`), así que la duración por herramienta viene del cliente, no del daemon.

**P9. Corridas — Incorporar.** El criterio es el de 33 (`mcnemar_exact`, `crates/clawg-eval/src/bin/rag-eval/stats.rs:13`), con la tarea como unidad pareada y k repeticiones por tarea. La simulación de la investigación es aritmética y no depende del código: 24 tareas con k = 3 detectan un efecto grande (0,30 → 0,70) casi siempre, y A3 contra A4 probablemente quede «no distinguible». Hay que dejar escrito antes de correr que eso no es «no hay diferencia». Una sola hipótesis primaria: A3 contra A0 en tareas de conocimiento.

**P13. Automatización — Modificar.** La separación entre lo puro (lectura de tareas, materialización por brazo, parser de `stream.jsonl`, rúbrica determinista, estadística) y la E/S (siembra, corredor, `check`) es la que piden `openspec/config.yaml` y `.claude/CLAUDE.md`. Lo que se cambia:

- **No se crea una biblioteca `src/lib.rs`.** La investigación la justificaba por el uso real de un tercer binario, pero revision-eval ya fue el tercero y el proyecto decidió copiar con los casos fijados (`revision-eval/design.md:18`). Un cuarto binario sigue esa regla. Si se quiere la biblioteca, es un cambio de mantenimiento aparte, sin comportamiento, y no una condición de este.
- **Un binario nuevo, `agent-eval`**, al lado de los otros, sin dependencias nuevas (`serde_json` y `std::process`).
- **CI sólo corre lo que no gasta:** el parser con `stream.jsonl` grabados en el piloto, la materialización, la rúbrica determinista y una corrida en seco donde un script reproduce un `stream.jsonl`. Nada que llame al modelo.

**Riesgo:** el formato de `stream-json` cambia con las versiones de Claude Code. Mitigación: la versión de Claude Code se fija por campaña y se anota en cada `run.json`, y los fixtures del parser se regraban al subirla.

**P15. Piloto — Incorporar.** 6 tareas × (A0, A3, A4, A3′) × k = 2 = 48 corridas, que la investigación estima en ~19 USD con Sonnet 5.5 y ~31 USD con Opus 5.5 (peor caso 72 USD con un tope de 1,5 USD por corrida). Sus preguntas son las correctas: los nombres reales de los campos, los tokens reales, la varianza entre repeticiones (que fija k), la validez de la suite y cuántos `ToolSearch` preceden a las llamadas a clawg. Pide autorización de gasto antes de correr.

### Bloque 2. La siembra

**P2 — Modificar: sembrar con `clawg import`, no extendiendo `EvalIngest`.** El problema es real y la consecuencia está bien vista: sembrar «ahora» mete todo en RECENT y regala a A3 lo que un usuario real tendría que buscar. Pero la solución recomendada (un `SaveSpec` en `EvalIngest::send`) ya no es la más barata:

| Opción | A favor | En contra |
|---|---|---|
| **a. `clawg import --from clawg` de un export escrito por la tarea** | Cero código de producto. Pasa por el daemon real, re-embebe con su modelo y conserva fechas, accesos, tier, estado, clave, procedencia, sesión y vínculos (`format.rs:92-117`). Siembra también pares ya juzgados (línea `conflict`) y revisiones `superseded`, que la categoría C necesita | No corre el detector (`specs/portability/spec.md:77`): un par C se escribe explícito en el archivo, que para medir es más controlado |
| b. `EvalIngest::save` con `age_ms`, `user_stated`, `derived_from` y accesos | Corre el detector y el dedup | Cambia `eval_api`, que es contrato; el store hay que abrirlo con `EvalIngest` y después levantar el daemon encima |
| c. `UPDATE` de fechas con el daemon parado | Sin código | Se salta las invariantes (decaimiento, tier, pares). Descartada |
| d. Sesión 0 del agente | La más realista para guardar → recuperar | Mezcla la calidad del save con la recuperación. Sirve para las tareas D, no para sembrar |

**Recomendación:** la opción a para la campaña, con dos edades (vieja, 30 a 90 días, por defecto; reciente, menos de 7, en una fracción) y con `access_count` pensado (una decisión con 3 accesos entra al workspace por `tier.rs`). La b queda sólo si una tarea necesita que el detector corra durante la siembra. **Cómo se mide que funcionó:** el `additionalContext` del evento `SessionStart` capturado en el piloto coincide con lo que predice el código para cada edad.

### Bloque 3. Lo que el arnés destraba en el producto

**P3. El recordatorio de 30 minutos — Incorporar.** Es una limitación de la medición más que del producto: en `claude -p` ninguna sesión dura 30 minutos, así que A3 es «herramientas + `session-start` + protocolo», no «+ los dos hooks». Se declara en el informe y se suman 4 a 6 tareas R con `--resume` después de 31 minutos, como hizo la verificación de 25. Una variable para acortar el umbral sólo existiría en builds `testing`, que la campaña no usa. Si los datos piden mover el umbral, es un arreglo aparte.

**P4. Herramientas diferidas — Investigar.** La premisa ya no es plausible sino cierta: con la configuración por defecto, el modelo ve los nombres de las 13 herramientas y las instrucciones, y para llamar a cualquiera necesita antes un `ToolSearch`. Lo que no se sabe es si eso cambia algo: las instrucciones ya nombran `save_memory`, `search_memory` y `search_project` (`server.rs:2519-2534`). Se mide en el piloto con A3′ (`ENABLE_TOOL_SEARCH=false`) y contando los `ToolSearch` que preceden a cada herramienta de clawg. Si A3′ gana claramente, hay dos caminos y ninguno se decide ahora:

| Camino | A favor | En contra |
|---|---|---|
| `_meta["anthropic/alwaysLoad"]` en `search_memory`, `recall_context` y `save_memory` | Lo decide el servidor, para todos los usuarios | Cambia el contrato del agente: se bendice `agent_contract.json` y se mide con `rag-eval --agent-view`. Una herramienta de carga inmediata tiene tope de 4.096 caracteres de descripción en vez de 16.384 (página de MCP) |
| `setup` escribe `alwaysLoad` para el servidor entero | No toca el contrato | Carga las 13 de entrada (~19,7 KB de `agent_contract.json`) y sólo para quien usa `setup` |

La fila «Marcar como diferidas las herramientas que no son centrales» de «Ideas evaluadas» queda sin objeto: el cliente ya difiere todas por defecto. La pregunta que queda es la contraria.

**P8. Apertura duplicada — Posponer.** Es cierta, pero cuesta del orden de 500 tokens por sesión que se cachean, y tiene una función: si el servidor no conecta, el `CLAUDE.md` sigue teniendo la apertura. El riesgo de que se desalineen es bajo porque las dos salen del mismo binario y `setup` reescribe el bloque al cambiar de idioma. Se decide con los tokens de A3 del piloto.

### Bloque 4. Calidad de los saves

**P11. Rúbrica — Modificar.** Cada criterio sale de una línea del protocolo, y eso es lo que la hace defendible. Tres cambios:

- **Q3 («en inglés») está mal escrito para 0.68.0.** El protocolo pide el idioma de la preferencia (`protocol.md:81`, `protocol.es.md:83-85`). El criterio pasa a ser «en el idioma del perfil», y puede reusar el detector que ya existe (`clawg_core::memory::language::detect`, `.claude/CLAUDE.md`) en lugar de un umbral nuevo de palabras vacías.
- **Primero sólo lo determinista** (Q2, Q5, Q6, Q7, Q8, la parte determinista de Q9, Q13, Q14, Q15 y Q16). **El juez se pospone** hasta que haya volumen de saves que lo justifique: su calibración pide 60 a 100 saves etiquetados a mano y un kappa por criterio, que es más trabajo que el resto de la rúbrica.
- **Q2 necesita `identifier_tokens`**, hoy `pub(crate)` (`crates/clawg-core/src/memory/search.rs:892`). Exponerlo por `eval_api` es un cambio chico con uso real, como 48 expuso `join_pieces`.

Q16 («se puede encontrar») es el criterio que más importa y el único que liga la rúbrica con la eficacia: si Q2 y Q3 no predicen Q16, la regla del protocolo no hace lo que se esperaba.

### Bloque 5. Lo externo

**P10. Costo — Modificar.** El orden de magnitud (0,4 a 0,65 USD por corrida, 140 a 235 USD la campaña núcleo) es razonable como presupuesto, pero sale de supuestos sin medir (15 llamadas, 45.000 tokens de contexto) y de una tabla de precios cacheada en un skill, con el multiplicador de escritura de caché sin verificar. Se rehace con los tokens del piloto y con la página de precios del día. Queda abierta una tensión: la investigación pide correr con el modelo que se usa de verdad («otro modelo mide otro producto»), y M3 eligió un modelo de costo bajo por el número de corridas (`revision-eval/proposal.md:23`). Se propone que la hipótesis primaria corra con el modelo real y M3 con el barato, como ya está decidido.

**P12. Benchmarks públicos — Posponer.** La conclusión de la investigación se sostiene: ninguno mide lo que clawg necesita saber primero (conocimiento no escrito del propio repositorio, preferencias, decisiones, memorias viejas). Los dos más cercanos piden Docker de SWE-bench y cientos de sesiones pagas, y de uno no se encontró el código. MemoryAgentBench (FactConsolidation) se descarta para este fin: lo que mediría, la revisión de hechos, ya lo mide `revision-eval` con su set propio. Las fuentes de esta sección no se volvieron a verificar para esta exploración.

**P16. Telemetría de Claude Code — Posponer.** Aporta duraciones por herramienta que `stream.jsonl` no trae, pero pide un colector y no cambia ninguna decisión del piloto. Se reabre si el piloto muestra que la latencia de las herramientas pesa en el tiempo de pared.

### P14. Encaje — Modificar

Un cambio propio, **96 agent-eval**, con esta exploración. No va «antes de 52 y 44» porque esos ya se archivaron. Va **en paralelo al orden de ejecución**, como 68, porque no toca el producto y su único bloqueo es la autorización de gasto, y no compite con los cambios de rendimiento y de docs que ocupan los lugares 1 a 9. Absorbe M3 de 48 tal como quedó escrito, con sus umbrales, porque es una tarea D con clave sobre el mismo corredor: hacer dos corredores sería duplicarlo.

Lo que destraba, cada cosa con su número ya fijado o por fijar:

- las tres decisiones pendientes de M3 (búsqueda por clave, clave en `recall_context`, quitar reglas de la clave; `revision-eval/verify.md:123-129`);
- el umbral de 30 minutos (P3), `alwaysLoad` (P4) y la apertura duplicada (P8);
- la reapertura condicionada del modelo NLI de «Ideas evaluadas» («si los agentes dejan sin resolver la mayoría de las candidatas»): `conflicts_judged` (P7) es ese dato.

## Roadmap

| Fase | Qué | Gasto | Esfuerzo | Sale cuando |
|---|---|---|---|---|
| 0 | Decisiones abiertas de abajo; primera versión del fixture y de las 6 tareas del piloto; exports de siembra (P2a); corredor, parser y corrida en seco en CI (P6, P13) | Ninguno | Medio-Alto | La corrida en seco pasa en CI y la auditoría de la suite no encuentra trazadores en el fixture ni en su índice de docs |
| 1 | Piloto (P15): 48 corridas, más A3′ (P4) | ~20 a 35 USD, tope duro | Bajo | Los fixtures del parser se regraban con salida real, k y el costo se recalculan (P9, P10), y A0 falla las K y aprueba la N |
| 2 | M3 de 48 (unas 30 corridas de dos sesiones, modelo barato) y la segunda tanda de tareas | Del orden de 30 a 60 USD para M3; la campaña núcleo según P10 recalculado | Alto | M3 contesta sus tres decisiones contra los umbrales de `revision-eval/proposal.md:50-55` |
| 3 | Campaña núcleo (A0, A2, A3, A4) con la rúbrica determinista (P11) y las tareas R (P3) | Según P10 recalculado | Medio | Hipótesis primaria contestada con p < 0,05 o declarada no distinguible |
| 4 | Lo que los números pidan: `alwaysLoad`, el umbral del recordatorio, la apertura duplicada, el juez de la rúbrica, benchmarks externos | — | Cada uno, su cambio | Cada uno con su medición antes y después |

## Decisiones abiertas

- **Autorización y tope de gasto** del piloto, y después de cada campaña.
- **Modelo de la hipótesis primaria**: el que se usa de verdad (la investigación) o uno barato (como M3).
- **Idioma de la campaña núcleo**: el del perfil real (`es`), el default (`en`) o los dos con la mitad de corridas cada uno.
- **Versión fija de Claude Code** por campaña.

## Preguntas que llegan de 98, 99 y 100

Una investigación posterior, sobre la integración con Claude Code, dejó tres cambios en exploración que piden datos de este arnés antes de decidir. No suman brazos nuevos al piloto; son conteos sobre las mismas corridas o tareas que ya están en la segunda tanda:

| Pregunta | Para qué cambio | Qué se cuenta, y en qué tanda |
|---|---|---|
| ¿Cuántas sesiones llegan a compactar sin haber guardado un `session_summary`? | [session-lifecycle](../session-lifecycle/exploration.md) (99), parte B | En el stream de cada corrida larga y en las tareas R: eventos de compactación y saves de `session_summary` antes de cada uno. Campaña núcleo |
| ¿Un resumen del cliente (`compact_summary`) sirve tanto como uno del agente para retomar? | 99, parte C | Tareas R con dos siembras de LAST SESSION, una con cada resumen. Segunda tanda |
| ¿Los subagentes llaman a las herramientas de clawg? | 99, parte D | Llamadas a `search_memory` y `search_project` dentro de un subagente. Piloto, sin costo extra |
| ¿Un aviso `changed=RUTA` hace que el agente compruebe el código antes de usar la memoria? | [memory-anchors](../memory-anchors/exploration.md) (98), paso 2 | Una tarea S con una memoria anclada a un archivo que la tarea cambió, con y sin el aviso. Segunda tanda, después del paso 2 de 98 |
| ¿Clawg agrega algo sobre la memoria nativa con los mismos hechos? | [native-memory-import](../archive/2026-10-31-native-memory-import/exploration.md) (100), parte B | Es A3 contra A4, la pregunta de producto de P1: 100 no recomienda apagar la memoria nativa hasta tenerla |
| ¿Buscar en `UserPromptSubmit` y mostrar hasta 3 entradas compactas ayuda más que lo que cuesta? | «Ideas evaluadas» del [README](../../README.md) | Un brazo de diagnóstico más, A3 con ese hook, sólo si el piloto muestra muchas tareas K sin ninguna búsqueda. No entra en el piloto |

## Hallazgos laterales

- **Las instrucciones quedan a 20 caracteres del tope viejo.** La apertura en inglés mide 2.028 de los 2.048 que `the_instructions_fit_older_clients` exige por los clientes anteriores a v2.1.296 (`server.rs:2571-2577`). Cualquier oración nueva en la apertura rompe ese test. Si la versión mínima de Claude Code soportada ya es 2.1.296 o posterior, el test puede pasar a 4.096; si no, la apertura no tiene margen. No se verificó qué versión mínima se soporta.
- **`wilson` está en tres binarios.** Es la decisión de copiar de revision-eval, no un descuido; anotarlo para cuando un cuarto binario lo necesite.

## Fuentes

- Código y specs de `develop` en `0f09b2b` (0.68.0), con las rutas y líneas citadas en cada punto.
- `openspec/changes/archive/2026-10-21-revision-eval/` (`proposal.md`, `design.md`, `verify.md`), para M3 y la regla de copiar.
- https://code.claude.com/docs/en/mcp, secciones «Scale with MCP tool search», «Mark a tool to load upfront or stay deferred» y «Configure tool search», leídas el 2026-10-31: todas las herramientas MCP diferidas sin `ENABLE_TOOL_SEARCH`, el modo `auto` con umbral del 10 %, `anthropic/alwaysLoad` (v2.1.285+), el tope de 4.096 caracteres para instrucciones y herramientas de carga inmediata y de 16.384 para las diferidas, y el de 2.048 antes de v2.1.296.
- La investigación previa sobre 0.27.0: los benchmarks (P12), la simulación de potencia (P9), los precios (P10) y las páginas de Claude Code que cita sobre `claude -p`, hooks, memoria y costos se toman de ella y no se volvieron a verificar aquí, salvo la de MCP.
