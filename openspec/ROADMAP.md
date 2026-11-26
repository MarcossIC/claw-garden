# Roadmap

Qué sigue y en qué orden. Es el único lugar que ordena lo pendiente: [README.md](README.md) es el mapa de lo que existe y cada cambio guarda su detalle en su carpeta. Lo terminado sale de acá.

**La etapa anterior está cumplida:** clawg es la única memoria de Claude Code desde el corte de Engram del 2026-11-20 (abajo), y Cursor tuvo 0 sesiones en el piloto.

**La meta de esta etapa:** clawg funciona bien todos los días, en todos los proyectos, sin funciones nuevas. Se cumple con dos semanas seguidas de uso normal en las que:

- el resumen de la sesión anterior llega entero en al menos 95 de cada 100 contextos de arranque (en el piloto, 116 de 250, por `context.cut`);
- `clawg doctor` no da errores y `ingest.failed` queda en 0;
- más de la mitad de las sesiones que guardan algo dejan su `session_summary` (hoy, 27 de 122, por `clawg metrics sessions`).

Lo que no acerca esa meta no se abre, aunque sea buena idea: va a [ideas.md](ideas.md). **Como mucho dos cambios abiertos a la vez**, sin contar los dormidos. Los cambios que no servían a esta meta se cerraron sin código el 2026-11-22 (abajo).

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
- al abrir una sesión, el contexto trae entero el resumen de la sesión anterior (lo arregló [conflicts-on-demand](changes/archive/2026-11-23-conflicts-on-demand/proposal.md), 141, archivado el 2026-11-23; falta medirlo con uso real, en su [verify](changes/archive/2026-11-23-conflicts-on-demand/verify.md#pendiente-en-local));
- `clawg doctor` no da errores y `ingest.failed` queda en 0;
- una memoria guardada desde Cursor aparece en Claude Code y al revés, en el proyecto correcto (sale del piloto: Cursor tuvo 0 sesiones en tres semanas);
- ninguna ventana se abre sola;
- la RAM es aceptable: esta semana decide el brazo B de ram-footprint-gates, porque cada evento de hook despierta el modelo.

## En la nube mientras corre el piloto

**Hecho:** [summary-on-stop](changes/archive/2026-11-23-summary-on-stop/plan.md) (142), archivado el 2026-11-23: cuando el agente de Claude Code termina un turno de una sesión que guardó algo sin `session_summary`, clawg se lo pide una sola vez y lo deja terminar. Queda en local, después de publicar, correr `clawg setup claude-code` otra vez y leer en `clawg metrics sessions` cuántas sesiones dejan resumen.

**Hecho:** [cursor-review](changes/archive/2026-11-26-cursor-review/plan.md) (144), archivado el 2026-11-26, que sale en 0.96.0. Es una función nueva que la etapa no traía, pedida para aprovechar los modelos de una suscripción de Cursor en la revisión. `clawg review` lanza el CLI de Cursor como revisor: lee el código y la memoria del proyecto y no escribe. Su sesión queda registrada con sus llamadas, sus tokens, su modelo y su veredicto. Un cambio mediano lo revisa sólo `clawg review`, y uno grande `change-reviewer` y `clawg review` en paralelo. Contar los tokens de Claude Code, la otra mitad de la comparación, queda como un cambio aparte que retoma [session-tokens](changes/archive/2026-11-22-session-tokens/exploration.md).

[session-usage](changes/archive/2026-11-11-session-usage/proposal.md) (121), partes A, B, C y D, archivado el 2026-11-11, sale en 0.81.0 ([verify](changes/archive/2026-11-11-session-usage/verify.md)). Va ahora y no después del corte porque mide: lo que entre en una versión al empezar la semana deja contadas las sesiones del piloto. El conteo dice cuántas sesiones buscan, cuántas guardan, cuántas cierran con resumen y cuántas compactan, el dato que agent-eval y la parte B de session-lifecycle esperan para decidir. No toca `memory.db`, así que subir una versión durante el piloto no arriesga las memorias. La parte D entra en la misma versión: quien ya corrió `setup` lo corre otra vez para tener el hook de cierre. Lo que sólo se ve con los clientes del piloto pasó al párrafo del piloto. [store-boundary](changes/archive/2026-11-12-store-boundary/proposal.md) (104) se implementó en la nube durante el piloto, cuando session-usage, el otro cambio que tocaba `crates/clawg-cli/src/daemon/api.rs`, ya estaba en `develop`. Su PR entró a `develop` el 2026-11-12 y el cambio está archivado ese día, y sale en 0.84.0 ([verify](changes/archive/2026-11-12-store-boundary/verify.md)).

[verdict-on-replace](changes/archive/2026-11-12-verdict-on-replace/proposal.md) (128) se hizo en la nube: son textos del puente y sus tests, y no toca `memory.db`. Abarató cada sesión que guarda: sacó el pedido al modelo que sólo juzgaba pares `compatible`. Está archivado el 2026-11-12 y sale en 0.85.0 ([verify](changes/archive/2026-11-12-verdict-on-replace/verify.md)). Lo que sólo se ve con uso real, cuántos pedidos siguen juzgando y cuántos reemplazos se detectan, queda por medir en local sobre los transcripts de las semanas siguientes, después de correr `clawg setup` otra vez en cada cliente ([pendiente](changes/archive/2026-11-12-verdict-on-replace/verify.md#pendiente-en-prod)).

[agent-eval-sessions](changes/archive/2026-11-17-agent-eval-sessions/proposal.md) (129) fue acá porque mide y no toca el producto: cambia sólo `crates/clawg-eval`, que ningún binario publicado enlaza, y no abre `memory.db`. Está archivado el 2026-11-17 junto con el arnés que amplía, [agent-eval](changes/archive/2026-11-17-agent-eval/proposal.md) (96), y los dos salen en 0.88.0 ([verify](changes/archive/2026-11-17-agent-eval-sessions/verify.md)). Su campaña paga corrió en local, con tope de 30 USD, y decidió lo que se construye después: el relevo entre sesiones rinde bajo `a3` en el régimen medido, y la línea base de repeated-errors (131) quedó escrita ([la campaña](changes/archive/2026-11-17-agent-eval-sessions/verify.md#la-campaña)). Lo que quedó sin medir (la celda C de la escala, el relevo a 150.000 tokens o más y el `session_summary` que ningún brazo guardó) está en su [pendiente](changes/archive/2026-11-17-agent-eval-sessions/verify.md#pendiente-después-de-publicar).

[protocol-block](changes/archive/2026-11-17-protocol-block/proposal.md) (132) fue en la nube: el sello del bloque, `doctor`, el aviso de `session-start`, `--protocol` y el brazo `a3-short` se probaron en Linux con el embedder falso y no tocan `memory.db`. Fue ahora porque corrige un daño de cada sesión: el bloque viejo sigue pidiendo veredictos `compatible`, el 97 % de los `resolve_conflict`. Está archivado el 2026-11-17 y sale en 0.88.0 ([verify](changes/archive/2026-11-17-protocol-block/verify.md)). La campaña paga sobre las tareas largas de agent-eval-sessions (129) corrió con la regla ya fijada y dejó el valor por defecto del bloque en `full` ([la campaña](changes/archive/2026-11-17-protocol-block/verify.md#la-campaña)). Falta, después de publicar, probar el bloque en una máquina con la versión instalada y medir el efecto en las sesiones ([pendiente](changes/archive/2026-11-17-protocol-block/verify.md#pendiente-después-de-publicar)).

[repeated-errors](changes/archive/2026-11-19-repeated-errors/proposal.md) (131) está archivado el 2026-11-19 y sale en 0.90.0 ([verify](changes/archive/2026-11-19-repeated-errors/verify.md)). Para tener los dos hooks nuevos, quien instaló a nivel usuario corre `clawg setup claude-code --profile P` otra vez, con el mismo perfil con que instaló, y quien instaló en un proyecto con `--local`, como el piloto, corre `clawg setup claude-code --local --profile prod` en ese proyecto ([pendiente](changes/archive/2026-11-19-repeated-errors/verify.md#pendiente-después-de-publicar)).

[contract-diet](changes/archive/2026-11-20-contract-diet/proposal.md) (133) está archivado el 2026-11-20 y sale en 0.92.0 ([verify](changes/archive/2026-11-20-contract-diet/verify.md)). El ruido del generador de esquemas salió del contrato sin cambiar ninguna palabra que el agente lee: 1.856 caracteres menos en las 13 herramientas. El bloque `lean` es ahora el valor por defecto, sin la apertura, que dan las instrucciones del servidor. Quien instaló a nivel usuario con el bloque entero y sin elegir variante recibe el aviso del bloque al empezar cada sesión, hasta que `clawg setup claude-code --profile P` lo reescribe como `lean` ([pendiente](changes/archive/2026-11-20-contract-diet/verify.md#pendiente-después-de-publicar)). Quien eligió una variante con `--protocol` la conserva. Sacar las secciones del resumen espera a memory-links (134).

## Corte (una noche local)

La reimportación del historial se hizo el 2026-11-08, antes del piloto ([history-reimport](changes/archive/2026-11-12-history-reimport/verify.md#el-corte-en-prod)). El resto del corte se hizo el 2026-11-20:

- **Claude Code:** `clawg setup claude-code` sumó los hooks `PostToolUse` y `PostToolUseFailure` de repeated-errors (131). El servidor ya estaba registrado como hace falta, así que no hizo falta `--remove`.
- **Engram salió de Claude Code:** se desinstalaron su plugin y su marketplace, y su bloque de protocolo salió del `CLAUDE.md` de usuario.
- **El flujo SDD** de los skills de usuario guarda ahora en `openspec` por omisión: ya existe y no depende de una herramienta de memoria.
- **No se importa nada más de Engram:** lo que guardó después de la reimportación queda en su archivo y no pasa a clawg. Se eligió cortar sin otra importación, y desde el corte Engram ya no recibe escrituras. Por eso salen el último export de Engram y el `--dry-run` de `clawg import --from claude-code`.
- **Engram no queda de respaldo:** sin escrituras, dos semanas instalado no guardarían nada que clawg no tenga, y la vuelta atrás de lo que está en clawg son sus propias copias (`pre-import`, `pre-undo`) y `clawg import --undo`.
- **Cursor no se toca:** tuvo 0 sesiones en el piloto, así que ni `clawg setup cursor` ni sacarle el servidor de Engram tienen nada que medir.

El paso que quedaba abierto, capturar en `Stop`, se decidió el 2026-11-22: lo hace [summary-on-stop](changes/archive/2026-11-23-summary-on-stop/exploration.md) (142), arriba. `SubagentStop` sigue sin hook.

## Dormidos: suben sólo si aparece su problema

Nadie los relee hasta que pase lo que los despierta.

| Cambio | Sube si |
|---|---|
| [tokenizer-footprint](changes/tokenizer-footprint/exploration.md) | La RAM del daemon molesta en uso real |
| [embedder-bakeoff](changes/embedder-bakeoff/exploration.md) | Aparece un modelo de embeddings candidato a reemplazar el actual |
| [model-reembed](changes/model-reembed/exploration.md) | Un modelo candidato pasa el umbral de embedder-bakeoff |
| [knn-prefilter](changes/knn-prefilter/proposal.md) | Las búsquedas en un proyecto chico traen poco o nada que sí está guardado. La fracción de filas ajenas es lo que decide la pérdida |
| [read-connections](changes/read-connections/exploration.md) | Un hook vence sus 3 s detrás de una operación larga y el arranque queda sin contexto |
| [session-lifecycle](changes/session-lifecycle/exploration.md) | Dos sesiones en paralelo del mismo proyecto reciben el traspaso de la otra |
| [topic-key-warning](changes/topic-key-warning/proposal.md) | No sube: se cierra cuando aparezcan casos reales que verifiquen su criterio |

## Cerrados sin código el 2026-11-22

Sus exploraciones quedan en `changes/archive/2026-11-22-<nombre>/`, con su fila en el [índice del archivo](changes/archive/README.md) y el motivo en su `state.yaml`. Se reabre uno desde ahí si su problema aparece.

- **Mejorar el orden de la búsqueda:** lexical-common-terms (137), lexical-stemming (139), memory-score-fusion (136) y docs-query-language (135). Se reabren si el piloto muestra búsquedas que no encuentran lo guardado.
- **Reorganizar el código sin cambiar lo que hace clawg:** storage-read-side (108), storage-write-side (109), search-index-ports (110) y storage-ports (111). La cadena de arquitectura ya hecha (store-boundary, memory-module-layout, core-use-cases y domain-types) queda como está.
- **Funciones o mediciones que la meta no pide:** memory-anchors (98), memory-links (134), project-groups (123), project-concept-map (125), project-architecture-map (126), synthesis-groups (138), stale-conclusions (140), session-tokens (122) y claude-code-plugin (120).

## Cómo se mantiene

- Al cerrar una noche, lo hecho sale de acá y lo que se cruzó se anota en la exploración de su cambio.
- Un cambio nuevo entra sólo si acerca la meta y hay lugar bajo el tope de dos abiertos. Si no, la idea va a [ideas.md](ideas.md) y no abre carpeta.
- Los prompts de cada noche no viven acá: son notas personales que apuntan a estos cambios.
