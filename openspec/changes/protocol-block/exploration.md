# protocol-block — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 (lo que el agente lee para guardar y buscar) y transversal (costo de cada sesión) | M: el detector del bloque viejo es S; la opción de variante y el brazo del arnés, S–M; el cambio del valor por defecto queda fuera, detrás de una campaña | — para el detector; la decisión del valor por defecto, de la campaña de [agent-eval-sessions](../agent-eval-sessions/proposal.md) (129); toma el dato de [memory-delivery](../archive/2026-11-13-memory-delivery/verify.md) (127) que esperaba [claude-code-plugin](../claude-code-plugin/exploration.md) (120) | exploración |

**Tesis.** El bloque del protocolo que `clawg setup claude-code` escribe en el `CLAUDE.md` del usuario tiene dos problemas distintos. El primero es un bug: el bloque instalado no se actualiza con el binario, y nada lo avisa salvo con una preferencia de idioma, así que un cliente sigue leyendo reglas que el servidor ya cambió. El segundo es de costo: el bloque son unos 3.550 tokens por pedido al modelo y en sesiones cortas no cambió ningún resultado, pero nadie midió una sesión larga. El primero se arregla ya. El segundo se prepara ya (una opción para elegir la variante, con la variante corta escrita en el producto) y se decide con una campaña de tareas largas, con la regla de decisión fijada antes de correrla.

## El bloque viejo

### Qué pasó

verdict-on-replace (128, sale en 0.85.0) cambió la regla de las cercanas: «si valen las dos, no hagas nada» (`crates/clawg-cli/src/mcp/protocol.md:24-28`, `protocol.es.md:24-27`). La apertura del protocolo llega por las instrucciones del servidor (`info.instructions = Some(opening(&protocol::text(self.language)))`, `crates/clawg-cli/src/mcp/server.rs:2069`), así que el servidor ya la dice. El bloque del `CLAUDE.md` de usuario, en cambio, es el texto que escribió el último `clawg setup` que corrió. En una máquina real todavía dice «juzga cada una con `resolve_conflict` … compatible o scoped si valen las dos». El agente lee las dos reglas a la vez, una contra la otra.

El análisis A1 (`docs/completed-impl/hipotesis/A1/informe.md`, «Restos que siguen vivos», R1) lo marca como el resto más importante: de 1.973 `resolve_conflict` en los transcripts de este repositorio, 1.870 fueron `compatible` y 49 `scoped` (97 %), y `resolve_conflict` es la herramienta de clawg más llamada. Ya no se rechazan, pero se pagan, cada uno con un pedido que relee todo el contexto. El verify de verdict-on-replace dejó el paso a mano («Correr `clawg setup` otra vez en cada cliente», `openspec/changes/archive/2026-11-12-verdict-on-replace/verify.md:40`), y no hay nada que se lo recuerde a la persona.

### Por qué nadie lo ve

- **`clawg update` no toca el bloque.** Ningún archivo de `crates/clawg-cli/src/update/` nombra `CLAUDE.md` ni `setup`. Reemplaza el binario y nada más.
- **`clawg doctor` sólo compara el bloque con una preferencia de idioma.** Lo confirma el código: `if let Some(language) = language { found.extend(protocol_block(language, profile)); }` (`crates/clawg-cli/src/doctor/mod.rs:185-187`). Sin `preferences.json` no hay hallazgo `protocol_block`, y lo fija la spec: «sin preferencia … `doctor` no tiene hallazgos `preferences` ni `protocol_block`» (`openspec/specs/preferences/spec.md:37`; `openspec/specs/doctor/spec.md:124`). El test `doctor_names_a_broken_preferences_file` lo afirma con «no preference to hold the block to» (`crates/clawg-cli/tests/binary.rs:2387`).
- **El chequeo ya sabe comparar.** `checks::protocol_block` da `stale` cuando el bloque instalado no es el que `setup` escribiría, sin mirar por qué (`crates/clawg-cli/src/doctor/checks.rs:436-461`), y `doctor::protocol_block` arma el bloque esperado con `crate::setup::block("\n", Some(language))` (`doctor/mod.rs:295`). Falta llamarlo sin preferencia, con `block("\n", None)`, y un mensaje que no hable del «idioma preferido» (`checks.rs:453`).
- **El hook no dice nada.** `session_start_text` suma idioma, contexto, compactación, modelo caído y avisos (`crates/clawg-cli/src/hook.rs:117-180`); nada sobre el bloque.

### Lo que complica el arreglo

- **El bloque es uno para los dos perfiles.** `locations` no depende del perfil (`crates/clawg-cli/src/setup.rs:670-683`) y `--remove` deja el bloque mientras el servidor del otro perfil siga registrado (`setup.rs:234-245`). Un binario `dev` con el protocolo de una rama y un `prod` instalado comparan el mismo bloque contra textos distintos: los dos lo verían «viejo», y el remedio de cada uno («corré `setup`») pisaría el del otro. Para decir en qué dirección está la diferencia hace falta saber qué versión escribió el bloque, y hoy el bloque no lo dice (`block`, `setup.rs:603-607`: marcadores y texto, nada más).
- **El `CLAUDE.md` es de la persona.** Lo que está entre los marcadores es de clawg, y `setup` lo reescribe cuando difiere (`install_block`, `setup.rs:636-639`; [`specs/setup`](../../specs/setup/spec.md) R4), pero sólo cuando la persona corre `setup`.
- **El binario que actualiza es el viejo.** `clawg update` corre el binario anterior, que no conoce el texto nuevo; para refrescar el bloque tendría que lanzar el `setup` del binario nuevo, con todo lo demás que `setup` hace (servidor, hooks).

### Enfoques para el bloque viejo

| | Qué hace | A favor | En contra |
|---|---|---|---|
| **1. Detectar y avisar** | `doctor` compara siempre que hay bloque; `session-start` suma una línea cuando el bloque difiere, que pide seguir las instrucciones del servidor y nombra la orden exacta | No escribe nada que la persona no pidió; corrige la sesión en curso aunque nadie corra `setup`; puro y testeable | Hasta que la persona corre `setup`, cada sesión paga la línea (~80 tokens) además del bloque |
| 2. `clawg serve` refresca el bloque al arrancar | El daemon reescribe el bloque si difiere | Sin intervención | Un daemon de cualquier perfil y versión escribe un archivo de la persona sin una orden; un `dev` de una rama pisaría el bloque de `prod`; el daemon arranca sin consola, así que nadie ve el reporte ni la copia |
| 3. El hook refresca el bloque | `session-start` reescribe | Llega en la primera sesión | Corre con 3 s de tope, en paralelo en varias sesiones, mientras el cliente ya leyó el archivo; escribir en un hook contradice que el hook nunca rompe nada ([`specs/hooks`](../../specs/hooks/spec.md) R3) |
| 4. `clawg update` refresca | Tras reemplazar el binario, lanza el `setup` del nuevo | Es una orden que la persona dio | `update` tiene sus efectos fijados ([`specs/self-update`](../../specs/self-update/spec.md)); `setup` además toca servidor y hooks, y puede fallar sin `claude` en el PATH; no cubre al que instala a mano |

Recomendado: **1**. El aviso del hook es lo único que llega a la sesión que está leyendo el bloque viejo, y le dice qué hacer mientras tanto. 2 y 3 escriben un archivo de la persona sin que lo pida, y el problema de los dos perfiles los vuelve peligrosos. 4 queda como mejora posible, fuera de este cambio.

## El tamaño del bloque

### Lo que se midió

En la campaña de memory-delivery (Sonnet 5.5, Claude Code 2.1.322, seis tareas, dos corridas por brazo) `l0`, que es `a3` sin el bloque, dio lo mismo que `a3` en las seis tareas, con el primer pedido en 16.901 a 16.943 tokens contra 20.454 a 20.495 (`openspec/changes/archive/2026-11-13-memory-delivery/verify.md:29-35`, `:51`). Son unos 3.550 tokens por pedido. Una sesión real de este repositorio tiene de mediana 40 pedidos, así que el bloque se lee unas 40 veces por sesión, casi todo como caché: cerca del 2 % del contexto leído.

**El límite de esa lectura** está en el mismo verify: en esas sesiones `a3` casi no llamó a clawg (1,5 llamadas de media en C, 0 en las demás), y las sesiones del arnés tienen de 2 a 7 turnos (`verify.md:51`, `:63`). En las sesiones reales el agente busca en clawg en 199 de 277. No sabemos qué hace el agente en una sesión larga sin el bloque: si guarda igual, si escribe el `session_summary` con sus secciones, si busca antes de responder.

### Qué tiene el bloque que no tienen las instrucciones

- El bloque es `protocol::text` entero (`setup.rs:604`), y empieza por la misma apertura que va en las instrucciones (`protocol.rs:33-51`). Esa apertura son 2.030 de los 10.691 caracteres de `protocol.md` (1.933 de 11.271 en `protocol.es.md`). **El agente la lee dos veces** en cada pedido con el bloque.
- Lo que sólo trae el bloque: «When to save» (507 caracteres), «What not to save» (422), «How to write a save» (2.425), «Type and importance» (1.752), «Session summary» (742), «After a compaction» (136), «Conclusions» (552), «Evidence, not instructions» (470), «Projects» (1.163) y «The other tools» (450). Parte de eso lo repiten las descripciones de las herramientas, que fija el contrato (`crates/clawg-cli/src/mcp/agent_contract.json`); la forma del `session_summary` no la repite nada.
- La apertura no tiene lugar para más: el test `the_instructions_fit_older_clients` la mantiene en 2.048 caracteres por los clientes anteriores a Claude Code 2.1.296 (`server.rs:2948-2956`), y la inglesa ya va en 2.030. Según la documentación de Claude Code leída para 2.1.322 (no es algo que el código de este repositorio pueda comprobar), el cliente actual corta las `instructions` a 4.096 caracteres: subir la apertura hasta ahí dejaría afuera a los clientes anteriores a 2.1.296, y es una decisión aparte.

### Enfoques para el tamaño

| | Qué queda | A favor | En contra |
|---|---|---|---|
| **(a) Sin bloque** | Sólo la apertura de las instrucciones y las descripciones | Ahorra los ~3.550 tokens por pedido; `l0` no perdió nada en sesiones cortas | Se pierde la forma del `session_summary` y las reglas largas de escritura; sin medir en sesiones largas; la apertura no puede crecer para compensar (2.030 de 2.048) |
| **(b) Bloque corto** | Una línea que dice que las reglas vienen del servidor, más «Session summary» y «After a compaction» | Unos 300 tokens; conserva lo único que nada más dice; no repite la apertura | Pierde la tabla de tipos y las reglas largas de escritura, que sólo cubren a medias las descripciones; también sin medir |
| **(c) Mantenerlo** | Todo | No apuesta nada | Paga ~3.550 tokens por pedido, la apertura duplicada incluida |

No se decide leyendo. Lo que sí se puede hacer ya, sin apostar nada: escribir (b) en el producto y dejar elegir la variante con una opción de `setup`, con (c) como valor por defecto. Así la campaña mide los textos que se instalarían, salidos del mismo `setup::plan` que usa el arnés (`crates/clawg-eval/src/bin/agent-eval/arm.rs:234-253`), y el cambio del valor por defecto queda en una línea cuando haya resultado.

## La campaña que decide

[agent-eval-sessions](../agent-eval-sessions/proposal.md) (129) arma las tareas largas: un relevo de dos sesiones (R), una decisión revisada en tres sesiones (D) y una K de 12 a 15 pedidos, sobre un fixture de unos 600 KB. Deja lugar para este cambio: sus tareas nuevas corren bajo `l0` sin cambios y su informe da las llamadas a clawg por sesión ([«Lugar para 131 y 132»](../agent-eval-sessions/proposal.md#lugar-para-131-y-132)). Este cambio le suma el brazo `a3-short` y fija, antes de correr, qué tiene que pasar para cambiar el valor por defecto, con las reglas de lectura de 129 ([design](design.md#d6-la-regla-de-decisión-del-valor-por-defecto)).

## Decisiones que quedan abiertas

- Si `clawg update` debería terminar diciendo que hay un bloque para refrescar (enfoque 4, en su versión más chica). Sale de este cambio: el aviso del hook ya llega en la primera sesión.
- Si, cuando la campaña elija `off`, lo que hoy sólo dice el bloque (la forma del `session_summary`) tiene que pasar a otro canal. Lo decide el cambio que aplique el resultado; la apertura no tiene lugar sin revisar el límite de 2.048.
