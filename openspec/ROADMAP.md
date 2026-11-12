# Roadmap

Qué sigue y en qué orden. Es el único lugar que ordena lo pendiente: [README.md](README.md) es el mapa de lo que existe y cada cambio guarda su detalle en su carpeta. Lo terminado sale de acá.

**La meta de esta etapa:** clawg es la única memoria de los clientes de agentes de uso diario, Claude Code y Cursor, en todos los proyectos. Lo que no acerca esa meta espera, aunque sea buena idea.

## Cómo se ordena

1. **Lo que daña si se instala antes, va antes.** Un contexto malo inyectado en cada sesión es peor que no inyectar nada.
2. **Primero un repositorio real, después todos.** Lo que se instala a nivel usuario afecta a todos los proyectos, y Cursor también lee esa configuración.
3. **Los datos se tocan una sola vez y al final,** después de ensayarlos sobre una copia.
4. **Lo que sólo vale si un problema aparece, espera a que aparezca.** Esos cambios tienen su condición escrita abajo.

## Local o nube

**Va en local sólo si necesita** alguna de estas cosas:
- el store real o una copia, un export o las transcripciones de los clientes;
- un cliente instalado, como Cursor con sus ventanas;
- una conducta de Windows que el CI no reproduce, o medir contra una línea base de esa máquina;
- instalar o publicar, que hace la persona.

**Todo lo demás va en la nube:** lo que se prueba en Linux con el embedder falso y termina en un PR a `develop`. Lo que un cambio de nube no pueda comprobar queda en su `verify.md` como pendiente en local.

Una noche lleva como mucho una tarea local y dos de nube que no toquen los mismos archivos.

## Piloto (una semana de trabajo real, no una noche)

Se publica lo anterior y se instalan los hooks de Claude Code en un repositorio real con memorias importadas, con `clawg setup claude-code --local --profile prod` corrido en ese repositorio ([setup-local-hooks](changes/archive/2026-11-08-setup-local-hooks/proposal.md)), que escribe sólo su `.claude/settings.local.json`, junto con Cursor en el mismo repositorio. El `clawg setup claude-code` de usuario no se usa todavía, porque escribe a nivel usuario. La excepción es Cursor, que se conecta con `clawg setup cursor --profile prod`, porque esa es la prueba con Cursor real ([pendiente en prod](changes/archive/2026-11-04-client-setup-cursor/verify.md#pendiente-en-prod)). Esa instalación también es a nivel usuario y afecta a cada ventana de Cursor, no sólo a este repositorio. Los mismos hooks prueban el arranque real de session-start-context ([pendiente en prod](changes/archive/2026-11-04-session-start-context/verify.md#pendiente-en-prod)), y que una preferencia declarada a Claude Code entre al arranque de Cursor en el mismo proyecto ([preference-model](changes/archive/2026-11-06-preference-model/verify.md#pendiente-en-prod)). Con 0.78.0 instalado, Claude Code en una carpeta lee lo de antes, y un save sin proyecto se rechaza y el agente lo repite con `project_path` ([bridge-project-identity](changes/archive/2026-11-07-bridge-project-identity/verify.md#pendiente-en-prod)). Con 0.81.0 instalada y los dos `setup` corridos otra vez para tener el hook de cierre, el piloto cuenta sus sesiones ([session-usage](changes/archive/2026-11-11-session-usage/verify.md#pendiente-en-prod)): que `sessionStart` y `sessionEnd` de Cursor real traen el mismo `session_id` y con qué motivo cierra una conversación, si `resume` sigue la fila de la sesión o abre una de otro id, y el reporte de la semana (`clawg metrics sessions --by project`, `--by agent` y `--by day`). Ese reporte dice cuántas sesiones cierran con `idle` frente a las que cierran por hook, y con eso se decide si hace falta vigilar el proceso del cliente.

**Pasa si:**
- responde 10 preguntas reales cuya respuesta sólo está en el historial;
- al abrir una sesión, el contexto trae entero el resumen de la sesión anterior;
- `clawg doctor` no da errores y `ingest.failed` queda en 0;
- una memoria guardada desde Cursor aparece en Claude Code y al revés, en el proyecto correcto;
- ninguna ventana se abre sola;
- la RAM es aceptable: esta semana decide el brazo B de ram-footprint-gates, porque cada evento de hook despierta el modelo.

## En la nube mientras corre el piloto

[session-usage](changes/archive/2026-11-11-session-usage/proposal.md) (121), partes A, B, C y D, archivado el 2026-11-11, sale en 0.81.0 ([verify](changes/archive/2026-11-11-session-usage/verify.md)). Va ahora y no después del corte porque mide: lo que entre en una versión al empezar la semana deja contadas las sesiones del piloto. El conteo dice cuántas sesiones buscan, cuántas guardan, cuántas cierran con resumen y cuántas compactan, el dato que agent-eval y la parte B de session-lifecycle esperan para decidir. No toca `memory.db`, así que subir una versión durante el piloto no arriesga las memorias. La parte D entra en la misma versión: quien ya corrió `setup` lo corre otra vez para tener el hook de cierre. Lo que sólo se ve con los clientes del piloto pasó al párrafo del piloto. [store-boundary](changes/store-boundary/proposal.md) (104) se implementa en la nube durante el piloto, ahora que session-usage, el otro cambio que tocaba `crates/clawg-cli/src/daemon/api.rs`, está en `develop`. Su PR espera al corte, así que no entra en ninguna versión del piloto.

[verdict-on-replace](changes/verdict-on-replace/proposal.md) (128) va en la nube: son textos del puente y sus tests, y no toca `memory.db`. Va ahora porque abarata cada sesión que guarda: saca el pedido al modelo que hoy sólo juzga pares `compatible`. Lo que sólo se ve con uso real, cuántos pedidos siguen juzgando y cuántos reemplazos se detectan, se mide en local sobre los transcripts de las semanas siguientes.

## Corte (una noche local, con el piloto aprobado)

La reimportación del historial ya se hizo, el 2026-11-08 y antes del piloto ([history-reimport](changes/archive/2026-11-12-history-reimport/verify.md#el-corte-en-prod)). Queda:

1. `clawg setup claude-code --remove` y `clawg setup claude-code`, porque el servidor ya registrado sin `--agent` no se actualiza solo. Después `clawg setup cursor`.
2. Sacar el plugin de Engram, su bloque de protocolo del `CLAUDE.md` global y su servidor de Cursor.
3. Decidir dónde guarda sus artefactos el flujo SDD de los skills de usuario: hoy usa Engram por omisión. Recomendado: `openspec`, que ya existe y no depende de una herramienta de memoria. No actualizar Engram antes del último export: clawg lee los formatos `0.1.0` y `0.2.0`.
4. Decidir si clawg necesita capturar en `Stop` y `SubagentStop`, donde hoy no instala hooks.
5. Mirar el `--dry-run` de `clawg import --from claude-code` y decidir.
6. Dejar Engram instalado y sin escrituras dos semanas, como vuelta atrás. Después, se desinstala.

## Sube sólo si el piloto lo pide

| Cambio | Sube si |
|---|---|
| [knn-prefilter](changes/knn-prefilter/proposal.md) | Las búsquedas en un proyecto chico traen poco o nada que sí está guardado. La fracción de filas ajenas es lo que decide la pérdida |
| [read-connections](changes/read-connections/exploration.md) | Un hook vence sus 3 s detrás de una operación larga y el arranque queda sin contexto |
| [session-lifecycle](changes/session-lifecycle/exploration.md) | Dos sesiones en paralelo del mismo proyecto reciben el traspaso de la otra |
| [agent-eval](changes/agent-eval/proposal.md) | El agente no busca ni guarda cuando debería, y hay que medirlo. Tareas, fixtures y corredor listos para sesiones pagas; falta correr el piloto en local, con una clave de API ([tareas](changes/agent-eval/tasks.md#después-de-este-cambio)) |
| [topic-key-warning](changes/topic-key-warning/proposal.md) | No sube: se cierra cuando aparezcan casos reales que verifiquen su criterio |

## Después del corte (no bloquean)

**Arquitectura, en este orden, porque cada paso abarata el siguiente:** store-boundary, que se implementa durante el piloto, entra con el corte, y la cadena sigue con memory-module-layout, core-use-cases, domain-types, storage-read-side, storage-write-side, y después search-index-ports y storage-ports, que tienen puerta: sin un candidato o una pregunta concreta, cierran sin código.

**Modelo y memoria del proceso:** tokenizer-footprint, embedder-bakeoff y model-reembed, que sólo sube si un modelo candidato pasa su umbral.

**Instalación y distribución:** [claude-code-plugin](changes/claude-code-plugin/exploration.md), que decide si servidor y hooks se distribuyen como plugin de Claude Code y si el bloque de protocolo de `CLAUDE.md` sobra. Acerca la meta, pero el piloto corre con `--local`: sube si instalar a mano en el segundo repositorio resulta engorroso.

**Calidad de lo guardado:** memory-anchors, y una política de olvido para `memory`, que hoy no existe y todavía no tiene cambio.

**Organización de los proyectos:** [project-groups](changes/project-groups/exploration.md) (123), que agrupa proyectos en un árbol para guardar y leer a nivel de grupo. Agrega una columna a `project`, así que va después del corte, cuando los datos ya se tocaron.

**Índice del proyecto, con prioridad baja:** [docs-index-status](changes/docs-index-status/exploration.md) (124) muestra qué indexa el daemon en segundo plano y cuánto le falta; es chico y va en la nube, cuando haya un lugar libre. [project-concept-map](changes/project-concept-map/exploration.md) (125) pregunta si vale registrar un mapa de conceptos y relaciones del proyecto, y empieza por una puerta en local, sin código. [project-architecture-map](changes/project-architecture-map/exploration.md) (126), el mapa visual y de arquitectura, espera a esa puerta. Ninguno de los tres acerca la meta de esta etapa.

**Costo por sesión:** [session-tokens](changes/session-tokens/exploration.md) (122) va en local, después de session-usage, y sólo si se quiere el costo de cada proyecto. Para saber si clawg ahorra alcanza agent-eval, que compara la misma tarea con y sin clawg.

## Cómo se mantiene

- Al cerrar una noche, lo hecho sale de acá y lo que se cruzó se anota en la exploración de su cambio.
- Un cambio nuevo entra en una etapa o abajo de todo, nunca en un lugar sin razón. Si no acerca la meta, va a «Después del corte».
- Los prompts de cada noche no viven acá: son notas personales que apuntan a estos cambios.
