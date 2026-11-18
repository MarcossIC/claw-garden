# contract-diet — Propuesta (la clase (b), el bloque `lean`)

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, y el costo de cada sesión | M, más una campaña paga chica | [protocol-block](../archive/2026-11-17-protocol-block/proposal.md) (132, archivado): el sello, las variantes y la regla [D6](../archive/2026-11-17-protocol-block/design.md#d6-la-regla-de-decisión-del-valor-por-defecto); [agent-eval-sessions](../archive/2026-11-17-agent-eval-sessions/proposal.md) (129): las tareas y las métricas | propuesta; la clase (a) ya está hecha ([exploración](exploration.md#hecho-la-clase-a)). Esta propuesta cubre el enfoque B1 y cierra D2, D3 y D6 de la exploración con su opción recomendada |

## Problema

1. **El bloque del protocolo es lo que más pesa en cada pedido** y casi todo lo lee sólo Claude Code. El bloque `full` son 10.691 caracteres en inglés y 11.271 en español (`crates/clawg-cli/src/mcp/protocol.md`, `protocol.es.md`), entre 3.550 y 4.000 tokens medidos, y viaja en cada pedido de la sesión (`assemble`, `crates/clawg-cli/src/setup.rs:689-697`). Fuera de la apertura, que son las instrucciones del servidor (`opening`, `crates/clawg-cli/src/mcp/protocol.rs:57`; `server.rs:2081`), ninguna sección repite literalmente otro texto; muchas dicen con otras palabras lo que ya dicen las descripciones de las herramientas ([exploración, «Repeticiones»](exploration.md#repeticiones)).
2. **La variante corta que existe pierde conductas.** `short` (`protocol::short`, `protocol.rs:118`) ahorró el 30 % de T y perdió B (buscar antes de editar) y J (`resolve_conflict` por save con cercanas) en la campaña de protocol-block ([verify](../archive/2026-11-17-protocol-block/verify.md#la-campaña)). Lo que quita es la apertura y ocho secciones juntas, así que no dice cuál de las partes sostenía cada conducta; lo único visible es que sin `topic_key` en el save aparecen cercanas y el agente resuelve a mano.

## Alcance

En `crates/clawg-cli` y en el arnés de `crates/clawg-eval`, en las dos lenguas:

- **`Variant::Lean`**, la cuarta variante del bloque, junto a `full`, `short` y `off`. Su texto es el título y la apertura de `full` **sin cambiar un carácter**, seguidos de las diez secciones de `full`, con sus mismos encabezados, comprimidas en dos archivos nuevos (`protocol.lean.md`, `protocol.lean.es.md`): cada regla queda, y sale sólo lo que ya dicen la apertura o las descripciones de las herramientas. La viñeta de `topic_key`, la regla del idioma del save y las secciones «Session summary» y «After a compaction» quedan enteras.
- **`full` no cambia.** `protocol.md` y `protocol.es.md` no se tocan, así que quien tiene `full` instalado sigue `current`.
- **El sello `protocol=lean`** en `setup claude-code`: `assemble` arma la variante, `freshness` la compara contra el texto de `lean` y `--protocol lean` la elige. Un binario que no conoce `lean` lee ese sello como ilegible y el bloque como `stale` (PB1 y PB2 de [`specs/setup`](../../specs/setup/spec.md) y [`specs/doctor`](../../specs/doctor/spec.md)); un test lo fija.
- **Los tests del protocolo valen para `lean`**: los de `server.rs` (`the_protocol_says_where_a_key_is_shown`, `the_protocol_describes_the_session_summary`, `the_spanish_protocol_describes_the_session_summary`, `the_protocol_names_only_what_exists`) y los de `protocol.rs` sobre las secciones, los nombres y los proyectos.
- **El brazo `a3-lean`** de `agent-eval`, junto a `a3-short`: `a3` con el bloque que escribe `setup --protocol lean`.
- `docs/reference/commands.md` (`--protocol`) y `docs/reference/testing.md` (el brazo).

## Fuera de alcance

- **Cambiar el valor por defecto.** Sigue en `full` hasta que la campaña corra con la regla de [design](design.md#la-regla-de-decisión) fijada antes. Lo aplica un commit propio después.
- **Correr la campaña.** Es paga: queda pendiente en [verify](verify.md), con su regla.
- **La apertura y las instrucciones del servidor** (D3, B4 de la exploración): `opening` no cambia, el contrato del agente (`agent_contract.json`, `agent_contract.es.json`) tampoco, y `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin `CLAWG_BLESS=1`.
- **Sacar o mover las secciones del resumen** (B2 y B3, D4 de la exploración): esperan a memory-links (134). `lean` las conserva tal como están en `full`.
- **Las descripciones de las herramientas** (B5): no cambian.
- El daemon, `session-start` y `doctor`: no cambian. `doctor` y el aviso del hook ya usan `freshness`, así que ven `lean` sin código nuevo.
- Cursor: no tiene bloque.

## Decisiones

Están en [design.md](design.md): `lean` es una variante nueva (D2), la apertura no se toca (D3), el umbral de ahorro (D5), `a3-lean` es un brazo del arnés (D6), el texto sale de la apertura de `full` y de dos archivos de secciones (D9), y qué se comprime y qué queda entero (D10), con la regla de decisión de la campaña.

## Cómo se mide el éxito

**Sin pagar**, con tests que hoy fallan:

- `protocol::lean` tiene la apertura de `full` byte por byte, sus diez encabezados en orden, la viñeta de `topic_key` y las dos secciones del resumen enteras, todos los nombres entre comillas invertidas de `full`, y ocupa como mucho el 70 % de `full` en las dos lenguas (`protocol.rs`).
- Los cuatro tests de `server.rs` sobre el protocolo pasan con `lean`.
- `setup --protocol lean` escribe el bloque con `protocol=lean`, una segunda corrida sin la opción no cambia nada, `freshness` lo da `current` y `doctor` lo da `ok` (`setup.rs`, `tests/binary.rs`).
- Un sello con una variante que el binario no conoce da el bloque `stale` (`setup.rs`).
- `a3-lean` es `a3` salvo el `CLAUDE.md` (`arm.rs`).
- El contrato del agente no cambia.

**El tamaño**, medido sobre los archivos: `lean` son 7.068 caracteres en inglés y 7.663 en español, el 66 % y el 68 % de `full` (−3.623 y −3.608). Con la regla de dedo de la exploración (unos 4,6 tokens de T por carácter de bloque) se esperan unos −16.600 tokens de T por sesión, cerca del 10,6 % de `a3`: menos que el 15 % que estimaba la exploración, porque la apertura, la viñeta de `topic_key`, la regla del idioma y las dos secciones del resumen quedan enteras y suman unos 4.300 caracteres.

**La decisión**, con la campaña y la regla de [design](design.md#la-regla-de-decisión): las seis condiciones de D6 de protocol-block y además T(`lean`) ≤ 0,9 × T(`a3`).

## Riesgos

- **El ahorro queda cerca del umbral.** El 10,6 % esperado es una estimación de la regla de dedo; la campaña puede dar menos del 10 % y entonces `lean` no pasa aunque no pierda ninguna conducta. Es lo que el umbral quiere: debajo, el ahorro no paga el riesgo.
- **Una regla comprimida pierde una conducta que el arnés no ve** (`new_project`, `user_answered`, `topic=K`): ver [design, D10](design.md#d10-qué-se-comprime-y-qué-queda-entero). Mitigación: cada regla con consecuencia sobre los datos queda; lo que sale es la explicación o lo que dice el esquema.
- **Las descripciones de las herramientas son diferidas** (`crates/clawg-eval/src/bin/agent-eval/arm.rs:437`): lo que `lean` deja en manos del esquema el agente lo lee recién cuando carga la herramienta. Para la herramienta que se usa, eso es antes de usarla.
- **Un binario anterior ve `lean` como `stale`.** Quien elige `lean` y vuelve a un binario anterior recibe el aviso del bloque y, si corre ese `setup`, el bloque vuelve a `full` sin `protocol=`. Es lo mismo que pasa con cualquier sello ilegible (PB1).
- **Se mide en inglés, se usa en español** (riesgo 8 de la exploración): la campaña corre con el bloque en inglés; `lean` en español hereda el resultado por analogía.

## Datos guardados

Ninguno: no cambia `memory.db`, el esquema ni ningún identificador. El único archivo que cambia es el `CLAUDE.md` del usuario, y sólo cuando la persona corre `setup claude-code --protocol lean`, con la copia en `backups/setup/` que `setup` ya hace. Para volver atrás: `clawg setup claude-code --protocol full`.
