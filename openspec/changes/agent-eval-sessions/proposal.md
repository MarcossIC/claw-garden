# agent-eval-sessions — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (1, 5) | M–L, sólo en `crates/clawg-eval` (el binario `agent-eval` y sus datos) | [agent-eval](../agent-eval/proposal.md) (96); sigue a [memory-delivery](../archive/2026-11-13-memory-delivery/verify.md) (127, archivado) | tareas 1 a 14 aplicadas y verificadas; la [segunda verificación](verify.md#segunda-verificación) pidió las enmiendas de las tareas 15 a 28, sin aplicar. La campaña paga no corre antes de ellas |

## Problema

La campaña de memory-delivery gastó 60 sesiones y no dio ganador. Su [verify](../archive/2026-11-13-memory-delivery/verify.md#lo-que-la-campaña-no-podía-medir) dice por qué: las sesiones del arnés tienen de 2 a 7 turnos y unos 17.000 tokens, con 16 memorias todos los brazos que entregan empatan, y una regla de decisión que nadie cumplió se mezcló con un artefacto del arnés (el orden del `MEMORY.md` de `a4`). Las sesiones reales, medidas en la [exploración](exploration.md#lo-que-se-midió-sobre-sesiones-reales), tienen 40 pedidos de mediana, pasan de 200.000 tokens en la mitad de los casos y se relevan con `/clear`.

El arnés no puede jugar nada de eso: una corrida es una sesión (`crates/clawg-eval/src/bin/agent-eval/arm.rs:187`, `run.rs:466-490`), una tarea tiene un prompt (`task.rs:34`), un trazador sólo nace en la siembra (`task.rs:176-179`), el ruido son 15 reglas, ningún brazo tiene shell (`arm.rs:316-320`) y de cada sesión se lee sólo el primer pedido (`stream.rs:158-163`).

Sirve a los objetivos 1 y 5: decide con medidas si el relevo, la escala y el ciclo de vida de una memoria son donde clawg hace rendir una sesión, antes de construir nada en el producto.

## Las hipótesis

| # | Hipótesis | Qué la mide |
|---|---|---|
| H1 | Relevo: cortar una sesión larga y seguir en una nueva es seguro, y con memoria cuesta poco | La tarea R jugada de las dos formas desde una misma primera parte, y la tarea D partida |
| H2 | Escala: con 300 reglas clawg sigue entregando y no cuesta más al arrancar | Las seis tareas del piloto y una K larga, con el ruido de 15 y el de 300 |
| H3 | Ciclo de vida: una memoria que escribió la sesión 1 llega a la última, y la versión revisada le gana a la primera | R y D, cuyos hechos nacen en un prompt y no en la siembra |
| H4 | El 0 de 2 de `a4` en C era el orden del archivo | C bajo `a4` con el orden corregido |
| H5 | Costo por llamada: cuánto agrega al contexto cada resultado de una herramienta de clawg | Todas las sesiones de la campaña, sin corridas propias |
| H6 | Error repetido: sin nada, la sesión siguiente repite el comando que no existe | La tarea E, línea base de repeated-errors (131) |

## Alcance

En `crates/clawg-eval`, sin tocar el producto:

- El `MEMORY.md` de `a4` va de la más vieja a la más nueva. Es lo primero y va solo.
- Una tarea puede tener varias sesiones, cada una con su prompt, sus trazadores (hechos que nacen en ese prompt) y sus checks. Una tarea con `prompt` sigue siendo una tarea de una sesión.
- Una corrida juega todas las sesiones sobre un mismo directorio de trabajo, home, store y memoria nativa, de tres formas: `split` (cada sesión es nueva), `joined` (cada una continúa la conversación) y `both` (después de la primera sesión la corrida se bifurca y juega la segunda de las dos formas).
- Cada sesión deja su stream, se puntúa por separado, y el resultado sigue a cada hecho de una sesión a la siguiente: si se guardó, si llegó, por dónde y si se usó.
- Los tokens de cada pedido y lo que agrega cada llamada; el sobrecosto de un relevo, en tokens.
- Lo entregado por la memoria nativa es lo que el archivo tenía antes de la sesión, cortado como lo carga el cliente.
- Un ruido con nombre por corrida y `noise-300.json`.
- Un check que cuenta fallos de una herramienta, una categoría `E` y un shell acotado a los comandos que la tarea nombra.
- La corrida en seco de varias sesiones, con las llamadas a clawg jugadas contra el servidor de la corrida.
- Cuatro tareas nuevas con sus fixtures y fuentes, y las de ejemplo que las prueban en CI.
- El reporte por forma, el ahorro de cada par y un informe por herramienta.

### Enmiendas antes de pagar

La [segunda verificación](verify.md#segunda-verificación) encontró tres puntos que sesgan lo que lee la regla de decisión y varios que la dejan leerse a mano. Entran en este cambio, antes de la campaña:

- Un hecho cuenta como guardado sólo si lo guardó una sesión anterior a la que lo usa (CRITICAL 1).
- El sobrecosto del relevo deja fuera las lecturas que un `Edit` exige, y la regla de H1 se reescribe sobre esa medida (CRITICAL 2; ver [H1](#h1-el-relevo)).
- Una sesión no puede leer fuera de su directorio de trabajo, y si lo logra, la corrida es un artefacto (CRITICAL 3; ver [abajo](#lo-que-una-sesión-no-puede-leer)).
- Los artefactos de la regla 3 los nombra `result.json` y los cuenta `report`, que además separa las filas por ruido y da en la tabla de pares la continuación, las compactaciones y las relecturas.
- Una sesión puede nombrar lo que no debe tocar todavía (`untouched`); la sesión 1 de R y las sesiones 1 y 2 de D lo usan.
- Un check que cuenta los intentos de un comando (`command_runs`), que la tarea E usa para H6.
- Toda corrida y toda rama que falla se puntúa, y la bifurcación restaura el home entero menos los modelos.

## Fuera de alcance

- El producto. Lo que gane se propone aparte: el aviso de corte y el resumen de relevo, o lo que diga 131.
- Correr la campaña. Es paga y local; su guía se escribe fuera de git, como las anteriores.
- El hook que aprende de los errores y su brazo, que son de repeated-errors (131), y la comparación de `a3` contra `l0` en tareas largas, que es de protocol-block (132). El arnés queda listo para los dos: ver [abajo](#lugar-para-131-y-132).
- La tarea de orientación (un mapa del proyecto). Su techo medido es del 1 al 2 % del costo de una sesión y no entra en el presupuesto sin sacarle corridas al relevo.
- Un corte en medio de una parte (`--max-turns`). Ninguna tarea de esta campaña lo usa; el corte cae entre dos pedidos de la persona, como un `/clear`.
- `l1` y `l2` con 300 reglas: el hook no puede entregar más de 10.000 caracteres. El arnés los rechaza en vez de truncar en silencio.
- Confirmar que a 150.000 tokens el corte no rinde. Pide una segunda tarea larga y no entra.

## Las tareas nuevas

Fixtures: `psf-requests` son los directorios `src/`, `tests/` (sin `tests/certs/`, que tiene enlaces simbólicos) y `docs/` (sin `docs/_static/`, que tiene imágenes) y los archivos `HISTORY.md`, `README.md`, `pyproject.toml`, `LICENSE` y `NOTICE` de https://github.com/psf/requests en `3c7baaa05ab4aa49cc819ce3a819d260de19c157`, Apache-2.0, sin cambios: 64 archivos, 601.855 bytes, 17.811 líneas, y ningún `CLAUDE.md` ni `AGENTS.md`. `shelf-notes` se escribe acá entero. Ninguna cadena de las que siguen está en esos archivos (comprobado sobre el árbol de ese commit); `agent-eval tasks` lo vuelve a comprobar.

| Tarea | Cat. | Fixture | Sesiones | Hechos y trazadores | Checks |
|---|---|---|---|---|---|
| `requests-timeout-decision-relay` | R | `psf-requests` | 2. La primera pide un inventario de cómo se manejan los timeouts, con archivo y línea, sobre todos los módulos de `src/requests/`, los tests, `docs/user/advanced.rst`, `docs/user/quickstart.rst` y `HISTORY.md`, escrito en `TIMEOUTS.md`; la persona dice ahí la decisión que se aplica después y pide no tocar el código todavía. La segunda dice que la revisión aprobó y pide aplicarla, documentarla y anotarla en `HISTORY.md` | Nacen en la sesión 1: `FORK_DEFAULT_TIMEOUT` (el nombre de la constante) y `4.5, 41` (su valor) | `src/requests/adapters.py` contiene los dos; `docs/user/advanced.rst` contiene `4.5, 41`; `HISTORY.md` contiene `FORK_DEFAULT_TIMEOUT`; `TIMEOUTS.md` contiene `adapters.py`. La sesión 1 deja sin tocar (`untouched`): `src/requests/adapters.py` y `HISTORY.md` sin `FORK_DEFAULT_TIMEOUT`, `docs/user/advanced.rst` sin `4.5, 41` |
| `requests-redact-decision-revised` | D | `psf-requests` | 3. La primera pide leer `src/requests/utils.py` y dice la decisión para una función que se agrega después. La segunda la revisa, sin repetir la anterior, y pide un test sin relación. La tercera pide implementar `redact_url_credentials` «como quedó decidido» | Sesión 1: `[hidden]` (la máscara de la primera decisión). Sesión 2: `_:_` (la de la revisada) | `src/requests/utils.py` contiene `def redact_url_credentials` y `_:_`, y no contiene `[hidden]`; `tests/test_utils.py` contiene `redact_url_credentials`. Las sesiones 1 y 2 dejan `src/requests/utils.py` sin `def redact_url_credentials` |
| `requests-history-owner-convention` | K2 | `psf-requests` | 1, de 12 a 15 pedidos: agregar `to_curl()` a `PreparedRequest`, documentarlo y anotarlo en `HISTORY.md` «con las convenciones de este fork» | Sembrado, 40 días, regla de `HISTORY.md`: cada entrada del fork termina con `(owner: resilience-guild)`. Trazador `resilience-guild` | `src/requests/models.py` contiene `def to_curl`; `HISTORY.md` contiene `resilience-guild` |
| `shelf-eza-missing` | E | `shelf-notes` | 3, con el shell acotado a `eza` y `ls`. El `CLAUDE.md` del fixture manda listar con `eza`, que no existe. Las dos primeras piden contar los archivos de `src/`; la tercera, los de `docs/` | Ninguno: se mide una conducta | En la sesión 3, la respuesta tiene el conteo, `command_runs` de `eza` es 0 (ninguna llamada a `Bash` lo invoca) y `tool_failed_count` de `Bash` con `eza: command not found` es 0 |

Lo que se toma de la fuente y lo que se escribe acá va en `data/agent-tasks/SOURCES.md`: de `psf-requests` sólo el fixture; los prompts, los hechos, las memorias, los checks y las edades son de acá. Las memorias de las seis tareas del piloto no cambian.

Dos tareas de ejemplo, sobre `example-tally`, prueban el arnés en CI: una de relevo de dos sesiones y una con un comando ausente.

## La regla de decisión

Se fija acá, antes de correr, y se lee con estas reglas:

1. **Sin «casi».** Un resultado que no cumple una condición no gana. No se agregan corridas para desempatar.
2. **Corridas por celda fijas:** las de la tabla del [presupuesto](#presupuesto). Una celda es una tarea bajo un brazo, una forma y un ruido.
3. **Artefactos.** Una corrida (o una rama de una bifurcación) se excluye, y se repite una sola vez, sólo por una de estas causas, que `result.json` nombra en `artifacts` sin interpretación y `report` cuenta en cada fila: (a) una sesión sin resultado o que terminó en error; (b) en un brazo con clawg, el servidor no figura `connected` en alguna sesión; (c) en la forma continua, el primer pedido de una sesión posterior tiene menos contexto que el último de la anterior (no continuó); (d) un comando que la tarea declara ausente estaba (una corrida paga se niega antes de empezar, así que no llega a un resultado); (e) una lectura fuera del directorio de trabajo que el cliente no negó ([abajo](#lo-que-una-sesión-no-puede-leer)); (f) una compactación en una sesión que no es la última de su corrida (en R, la sesión 1: el par ya no compara seguir con cortar, sino compactar con cortar); (g) una sesión que dejó en un archivo lo que la tarea le pide no tocar todavía (`untouched`): el relevo ya no se mide. Una continuación que arrancó con la caché fría (menos del 80 % de su primer pedido leído de caché) cuenta para el éxito y no para el costo. Una compactación en la última sesión cuenta para las dos cosas y se informa: es lo que hace una sesión real que sigue. Un intento de lectura fuera que el cliente negó no es artefacto; se cuenta y se informa.
4. **Si se excluyen más de 2 corridas en una fase, la fase se detiene** y se revisa el arnés antes de gastar más.
5. **Lo que es por construcción se dice.** Un resultado que el diseño de la corrida ya determina no se presenta como hallazgo.

### H1, el relevo

Se lee por brazo X, con `a3` y `a4` en esta campaña (`l0` lo corre 132 sobre la misma tarea). En R, cada corrida es un par: la misma primera sesión, y la segunda jugada continua y nueva.

Los pares que cuentan son los que no tienen artefacto en ninguna de sus dos ramas; un par con un artefacto se repite entero, una sola vez.

- **Validez:** en R, la forma continua aprueba en al menos 2 de los 3 pares de X, y el contexto al bifurcar es de 150.000 tokens o más de mediana; con menos contexto la regla se aplica igual y el resultado se informa como «régimen de N tokens». En D, la forma continua bajo `a0` aprueba 2 de 2. Una tarea que no es válida no decide: se informa como rota y H1 queda sin veredicto para ese brazo.
- **Seguro:** en R, ningún par en que la forma continua aprueba y la nueva falla; y en D, la forma partida aprueba 3 de 3.
- **Sobrecosto evitable:** la mediana, en los pares de R, de lo que el relevo le suma a la sesión nueva y una memoria puede ahorrar, en tokens: lo que su primer pedido tiene de más sobre el primer pedido de la sesión 1 (sin bajar de 0), más lo que relee a 4 bytes por token. Relee: los `Read` de un archivo que la sesión 1 leyó o escribió, salvo los de un archivo que la misma sesión nueva edita (antes o después de leerlo) y que la sesión 1 no escribió; las llamadas a `search_memory`, `recall_context`, `read_memory` y `list_memories`; y los `Read` del directorio de la memoria nativa. Un archivo que la sesión 1 escribió (en R, `TIMEOUTS.md`) cuenta siempre: es el camino del relevo por el repositorio.
- **Veredicto de X:** `rinde` si es seguro y su sobrecosto evitable es de 10.000 tokens o menos; `seguro pero caro` si es seguro y pasa de 10.000; `no es seguro` en otro caso.

**Por qué se mide así.** La sesión 2 de R tiene que editar `src/requests/adapters.py`, `docs/user/advanced.rst` y `HISTORY.md` (134.458 bytes entre los tres, unos 35.000 tokens leídos enteros), y el cliente exige un `Read` en la misma sesión antes de un `Edit`. Esas lecturas las paga cualquier sesión nueva, con memoria o sin ella, y su tamaño es de la tarea: contadas, el veredicto lo decidía si el agente lee por rangos. Fuera de la medida, lo que queda es lo que la sesión nueva tuvo que redescubrir, que es lo que una memoria puede ahorrar. La regla se lee igual en toda corrida de dos sesiones, con o sin pareja, porque se calcula del stream de cada rama y no de la diferencia entre las dos.

**Por qué 10.000.** Con la caché leída a 0,05 veces la entrada (Sonnet 5.5 y Opus 5.5), cortar a 200.000 tokens ahorra a lo sumo el 15,1 % del costo, y 10.000 tokens de sobrecosto dejan el 13,2 % ([tabla de la exploración](exploration.md#cuánto-ahorra-un-corte-según-el-precio-de-leer-la-caché)). La medida deja fuera las relecturas para editar, que en un relevo real también se pagan: en los pares de sesiones reales de la exploración, lo evitable de lo que la segunda relee antes de editar fue el 26 %, 1.740 tokens de media, así que lo inevitable rondó los 5.000. Con 10.000 evitables más esos 5.000 el corte conserva el 12,1 %, cuatro quintos del techo. El umbral anterior, 20.000 tokens con todo contado, pedía lo mismo en lo económico, pero R no lo podía cumplir por cómo está hecha. A 0,1 veces el corte ahorra el 23,2 % con 10.000; con una relación de 0,025 no ahorra, sea cual sea el veredicto, y el informe lo dice junto al veredicto.

**Lo que se informa y no decide:** las relecturas para editar de la rama nueva; las relecturas de la rama continua, medidas igual, sin el primer pedido (su primer pedido es la conversación entera); la diferencia entre las relecturas de las dos ramas de cada par; y el ahorro medido en cada par (1 menos el costo de la segunda sesión nueva sobre el de la continua, en USD del cliente), que depende de cuánto dura la segunda parte de esta tarea. La regla original de «15 % o más» está por encima del techo a los precios del modelo de la campaña.

**Los archivos de R no se achican.** Su tamaño es lo que lleva la sesión 1 a 150.000 tokens o más, la condición de validez; la medida nueva ya no depende de él; y el fixture es una copia sin cambios de su fuente (`data/agent-tasks/SOURCES.md`), cuyos trazadores se comprobaron sobre ese árbol. Recortarlo pediría otra fuente, otra comprobación y una sesión 1 más corta.

`a0` partida es el `/clear` sin nada. Si `a0` también es seguro y su sobrecosto no supera al de X, la memoria no aportó al relevo en estas tareas: se informa así y no se construye nada sobre este resultado.

Qué sigue: si `a3` rinde y `a0` no es seguro, se propone el aviso de corte y el resumen de relevo sobre lo que clawg ya tiene. Si `a3` no es seguro y `a4` sí, el cambio siguiente se diseña contra lo que hace la nativa. Si ninguno es seguro, el relevo necesita un resumen escrito antes del corte, y eso es una hipótesis nueva, no otra lectura de ésta.

### H2, la escala

Sobre `requests-history-owner-convention`, 3 corridas por celda, con 15 y con 300 reglas:

- **No medible:** si `a3` con 15 reglas aprueba menos de 2 de 3. El agente no tira de la memoria ni con el store chico, y la escala no es lo que limita.
- **clawg aguanta:** `a3` con 300 aprueba al menos tantas como con 15; en S, C y N del piloto con 300, `a3` y `l0` aprueban 2 de 2; y la mediana del primer pedido de `a3` con 300 no supera en más de 500 tokens a la de 15.
- **Ventaja en escala:** clawg aguanta y `a3` con 300 aprueba al menos 2 corridas más que `a4` con 300.

Lo que es por construcción: el `MEMORY.md` de `a4` con 300 reglas tiene unas 900 líneas, el cliente carga 200 y la memoria de la tarea, de 40 días, queda fuera. Que `a4` no la reciba al arrancar no es un hallazgo; lo que se mide es si el agente va a buscarla al archivo. Si el canario muestra que el cliente carga más que las 200 líneas, el modelo de carga se corrige antes de la fase 3 y esta salvedad cae.

**El canario.** N bajo `a4` con 300 reglas y con 15, una corrida cada una en el humo. Con 15 el `MEMORY.md` de N tiene 2.083 bytes y 46 líneas, y lo carga entero cualquier modelo de carga; con 300 tiene 33.431 bytes y 901 líneas, y sus 200 primeras líneas son 7.274 bytes. Δ es el primer pedido con 300 menos el primer pedido con 15. Si el cliente corta en 200 líneas, Δ ronda los 1.300 tokens; si corta en 25.000 bytes, unos 5.700; si carga el archivo entero, unos 7.800. **El modelo cuadra si Δ está entre 500 y 3.000 tokens.** Por debajo de 500 el cliente no cargó el archivo con 300 reglas; por encima de 3.000 cargó más que las 200 líneas. La comparación es entre dos corridas del mismo brazo y la misma tarea, así que las instrucciones de la memoria nativa, que `a0` no tiene, se restan solas.

### H3, el ciclo de vida

No tiene ganador propio: es la condición de seguridad de H1 con su causa. Cada hecho nacido en una sesión se clasifica en la última, por el resultado y sin juicio: `no guardado` (ninguna sesión anterior lo escribió en una herramienta de clawg ni en un archivo de la memoria nativa), `guardado y no llegó`, `llegó y no se usó`, `usado`. En D, además: `usó la primera` si el archivo tiene `[hidden]`. Un brazo **mantiene** un hecho si sale `usado` en 3 de 3.

Un guardado lo hace una sesión anterior a la última: lo que la última sesión guarda al terminar no trajo nada a ella. Un guardado se reconoce por el trazador literal en la entrada de la llamada, así que en R el hecho que decide es `FORK_DEFAULT_TIMEOUT`, un identificador que se guarda tal cual; `4.5, 41` se informa y no decide, porque un guardado que dice «4,5 s y 41 s» sale `no guardado`. En D deciden `[hidden]` y `_:_`.

### H4, el orden del `MEMORY.md`

C bajo `a4`, con 15 reglas y el orden corregido, 2 corridas. Se informa junto al 2 de 2 del piloto y al 0 de 2 de la campaña. No decide nada.

### H5, el costo por llamada

Por herramienta de clawg: llamadas, cuántas se pudieron medir solas, la mediana y el p90 de los tokens que agregan, y la mediana de los bytes de su resultado entre 4. Lo que agrega una llamada incluye el pensamiento del modelo en ese mensaje, así que no basta solo. Una herramienta con las dos medianas, lo que agrega y su resultado, de más de 1.500 tokens, el doble de lo que `search_memory` mide en agent-view (`openspec/config.yaml:28`), queda anotada como candidata a una dieta. No decide nada más.

### H6, el error repetido

Sobre `shelf-eza-missing`, 3 corridas por brazo. Un **intento** es una llamada a `Bash` cuyo comando invoca `eza` en cualquiera de sus partes (`command_runs`), haya dicho lo que haya dicho la salida: así H6 no depende de que el shell del cliente escriba `eza: command not found`. Ese texto se cuenta aparte (`tool_failed_count` con `text`) y se informa.

- **Hay efecto:** bajo `a0`, la sesión 3 intenta `eza` al menos una vez en al menos 2 de 3.
- **La memoria de hoy ya lo evita**, para `a3` o `a4`: 0 intentos en la sesión 3 en 3 de 3.

Si hay efecto y ninguna memoria lo evita, 131 tiene su línea base y su meta, 0 intentos en 3 de 3. Si no hay efecto, la tarea no mide lo que 131 necesita y 131 la cambia en su propuesta.

## Lo que una sesión no puede leer

Al lado de `work/`, en la raíz de la corrida, están `run.json` (la tarea entera, con sus prompts, trazadores y checks), los streams de las sesiones anteriores, `fork/` y, en la rama nueva, `joined/work/` con la decisión ya aplicada; en `claude-config/projects/` está la conversación de la sesión 1. Sin una regla, una lectura fuera de los directorios de trabajo sigue el modo de permisos; con `-p` y `acceptEdits` el cliente no documenta si se niega o pasa, y ningún brazo niega `Read`, `Grep` ni `Glob`. Una sesión que lee ahí obtiene la respuesta por un camino que no es la memoria, y `brought_by` diría `Read`, igual que si la hubiera leído en `TIMEOUTS.md`. Es la fuga de K3 en el piloto, más grande.

Se niega y además se detecta:

- **Se niega.** El `settings.json` que el arnés escribe en `claude-config/` para todo brazo (`arm.rs:356-357`) lleva `permissions.blockReadsOutsideWorkingDirectories: true`: el cliente niega `Read`, `Grep`, `Glob` y `LSP` fuera de los directorios de trabajo en todo modo. Bajo `a4`, el directorio de su memoria nativa va en `permissions.additionalDirectories`, para que el bloqueo no le niegue sus propias notas. La clave pide Claude Code 2.1.257 o posterior: una corrida paga se niega con una versión anterior.
- **Se detecta.** Cada sesión lista sus lecturas fuera del directorio de trabajo (y fuera de la memoria nativa en `a4`): las llamadas a `Read`, `Grep`, `Glob` o `LSP` cuya ruta cae fuera, y las de `Bash` cuyo comando nombra una ruta absoluta o con `..` que cae fuera. Una que el cliente negó se cuenta y no es artefacto; una que devolvió contenido es el artefacto (e). Quedan fuera los archivos que el cliente escribe para la propia sesión y le pide leer (los que llevan en la ruta el id de la sesión de su `init`).

La conversación de la sesión 1 sigue siendo el contexto de la rama continua, que la carga el cliente con `--continue` y no una herramienta: el bloqueo no la toca. Lo que impide es que la rama nueva la lea con `Read`.

El shell no entra en el bloqueo: lo que el cliente niega así son sus herramientas de archivo. Sólo E tiene shell, acotado a `eza` y `ls`, que no leen contenido, y E no tiene trazadores; un `ls` fuera de `work/` queda detectado igual.

## Presupuesto

Con Sonnet 5.5 y los precios de la exploración. Una sesión corta costó de 0,023 a 0,038 USD de media en la campaña anterior. Una larga se estima por sus tokens: escribir en caché lo que entra al contexto, leerlo en cada pedido siguiente y unos 500 tokens de salida por pedido.

| Corrida | Pedidos | Contexto | Estimado (USD) |
|---|---|---|---|
| Tarea del piloto | 2 a 7 | 17.000 a 21.000 | 0,03 |
| `shelf-eza-missing`, 3 sesiones cortas | 9 a 12 | 17.000 a 21.000 | 0,12 |
| `requests-history-owner-convention` | 12 a 15 | hasta 60.000 | 0,30 |
| `requests-redact-decision-revised`, 3 sesiones | 16 a 20 | hasta 60.000 | 0,40 |
| `requests-timeout-decision-relay`, bifurcada | 10 a 12 la primera parte; 10 a 12 cada segunda | 150.000 a 200.000 al bifurcar | 1,30 (0,65 la primera parte, 0,35 la continua, 0,30 la nueva, que relee unos 35.000 tokens para editar) |

| Fase | Corridas | USD |
|---|---|---|
| 0. Ensayo sin pago: todas las tareas en seco, con el modelo falso en CI y con el real en local | — | 0 |
| 1. Humo y canarios: R bifurcada bajo `a3`; D partida bajo `a3`; la K larga bajo `a3` con 300; E bajo `a0`; N bajo `a4` con 300 y con 15 (el canario) | 6 | 2,18 |
| 2. Relevo y ciclo de vida: R bifurcada bajo `a0`, `a3` y `a4`, 3 cada una; D continua bajo `a0`, 2, y partida bajo `a0`, `a3` y `a4`, 3 cada una | 20 | 16,10 |
| 4. Error repetido: E bajo `a0`, `a3` y `a4`, 3 cada una | 9 | 1,08 |
| 3. Escala: las seis del piloto con 300 bajo `a3`, `l0` y `a4`, 2 cada una; las seis bajo `a4` con 15 y el orden corregido, 2 cada una; la K larga bajo `a3` y `a4` con 15 y con 300, 3 cada una | 60 | 5,04 |
| **Total** | **95** | **24,40** |

Las fases corren en el orden de la tabla: la 4 antes que la 3, que es la que menos decide y la primera que se queda sin presupuesto.

**Los topes de cada proceso.** `--max-budget-usd` es un argumento de la corrida, y el cliente lo aplica a cada proceso sin contar lo gastado antes de continuar: vale por sesión, no por corrida. Se fija por tarea: 1,20 para R (la primera parte estimada en 0,65 y una segunda continua que arranca sobre 150.000 tokens), 0,50 para D y la K larga, 0,15 para E y las del piloto. El peor caso de una corrida es su número de procesos por su tope: 3,60 una R bifurcada, 1,50 una D, 0,50 la K larga, 0,45 una E y 0,15 una del piloto.

- **Tope: 25 USD, duro.** Una corrida empieza sólo si lo gastado hasta ahí, sumado con `agent-eval costs` sobre todas las corridas de la campaña (las que fallaron incluidas), más su peor caso, no pasa de 25. Si lo gastado más el estimado de lo que falta pasa de 25, se aplica el orden de recorte antes de seguir.
- Las corridas del humo cuentan en su celda si después del humo no cambia ni la tarea ni el arnés: las seis son celdas de las fases 2, 3 y 4, y entonces el total baja a 22,22. Con 24,40 el margen para repetir artefactos es de 0,60 USD (una D, no una R); con 22,22, de 2,78.
- **El humo es una puerta.** La campaña no sigue si la primera parte de R cuesta más de 1,00 USD; si la rama continua de R tiene el artefacto (c) o la sesión 1 compactó (f); o si alguna corrida del humo tiene el artefacto (e), porque el bloqueo de lecturas no funcionó. **El canario** es una puerta sólo de la fase 3: si Δ no está entre 500 y 3.000 tokens ([H2](#h2-la-escala)), la fase 3 no empieza hasta que el modelo de carga se corrige y se vuelven a puntuar sus corridas. Las fases 2 y 4 siguen, porque con 15 reglas el `MEMORY.md` de toda tarea tiene de 46 a 52 líneas y lo carga entero cualquier modelo.
- **Orden de recorte**, si el costo medio de una fase pasa del estimado en más de la mitad, o si lo gastado más lo que falta pasa de 25: la K larga baja a 2 corridas por celda; después D continua baja a 1; después las del piloto con 300 corren sólo bajo `a3` y `a4`. R no se recorta.
- La fase 3 es la que menos decide, porque parte de su resultado es por construcción: sin ella el total es 19,36.

## Cómo se ensaya sin pagar

`agent-eval run … --replay DIR` juega cada sesión con un stream escrito a mano. El cliente reproducido aplica sus `Write` y `Edit`, y desde este cambio manda sus llamadas a clawg al servidor de la corrida, por el mismo puente que usa el agente, e imprime lo que el servidor contestó. Así la sesión 2 «busca» de verdad lo que la sesión 1 «guardó»: el store, la siembra, el puente, los archivos por sesión, la bifurcación y el puntaje se prueban enteros sin un modelo. Lo que no se ensaya es si un agente guarda, busca u obedece, que es lo único que pide una sesión paga, ni los hooks del cliente, que la corrida en seco no corre.

## Lugar para 131 y 132

- **131 (repeated-errors)** necesita varias sesiones sobre un mismo home, un shell acotado y un check de fallos: son `sessions`, `shell` con `missing_commands`, `command_runs` y `tool_failed_count`. Su brazo con el hook nuevo es una variante más de `Arm`; nada del modelo de sesiones depende de la lista de brazos. `shelf-eza-missing` es su línea base, y 131 la puede cambiar en su propuesta.
- **132 (protocol-block)** compara `a3` y `l0` en tareas largas: las cuatro tareas nuevas corren bajo `l0` sin cambios, y el informe por herramienta le da las llamadas a clawg por sesión.

## Cómo se mide el éxito

- El cambio está hecho cuando pasan los tests de la [spec](spec.md), que hoy fallan, `agent-eval tasks` da válidas las tareas y los dos ruidos, y la suite entera pasa.
- La campaña aplica la regla de arriba tal como está escrita.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| La bifurcación restaura mal y una rama ve lo que hizo la otra | Un test juega una corrida bifurcada en la que la rama continua escribe un archivo y guarda una memoria, y comprueba que la nueva no ve ninguno de los dos |
| `--continue` no conserva el contexto en `-p` | Es el artefacto (c): lo muestra el primer par del humo, antes de la campaña |
| La memoria nativa no escribe en `-p` | El resultado cuenta los `Write` y `Edit` a su directorio en cada sesión. Si es 0 en todas, `a4` se informa como «no escribió» y no como «perdió» |
| La primera parte de R no llega a 150.000 tokens: el agente no lee todo | El humo lo mide; el prompt nombra cada archivo y pide archivo y línea. La regla se aplica igual y dice el régimen |
| El agente escribe la decisión en `TIMEOUTS.md` y el relevo va por el repositorio | Es un `/clear` real. El resultado dice por dónde llegó cada hecho, y la regla ya contempla que `a0` sea seguro |
| Un shell, aunque acotado, en una sesión sin nadie | Sólo los comandos que la tarea nombra, que deben ser de lectura. Una sesión ya puede leer con `Read`; el shell no le suma alcance |
| 600 KB de un proyecto ajeno entran en el índice de docs de este repositorio | Un `.ignore` en `fixtures/` los deja fuera del índice y de `rg`; la corrida copia el fixture, no ese archivo |
| El costo de una sesión larga está estimado, no medido | El humo mide una de cada tipo, y hay tope por proceso, tope total contra el peor caso de cada corrida y orden de recorte |
| Una sesión lee lo que la corrida guardó fuera de `work/` (la tarea, los streams, la otra rama) | El cliente bloquea las lecturas fuera de los directorios de trabajo, y la que pasa igual es el artefacto (e) |
| El bloqueo le niega a `a4` sus notas, o al agente un archivo que el cliente le pide leer | El directorio de la memoria nativa es un directorio de trabajo de `a4`; las lecturas negadas se cuentan por sesión, y el humo las muestra antes de la campaña |
| La sesión 1 de R aplica la decisión que se le pidió no aplicar, y las dos ramas aprueban sin relevo | `untouched` lo comprueba después de la sesión, y es el artefacto (g) |
| La sesión 1 de R compacta antes de bifurcar | Es el artefacto (f), y en el humo detiene la campaña: el par mediría compactar contra cortar |
| Una sesión que falla no deja resultado, y el gasto y las exclusiones se cuentan de menos | Toda corrida y toda rama se puntúa, también la que falla |

## Datos guardados

Ninguno. El arnés escribe sólo en el directorio de cada corrida. `run.json` y `result.json` ganan campos (con las enmiendas, `artifacts`, `rereads`, y en cada sesión `outside_reads` y `untouched`); los de corridas anteriores se siguen leyendo, y un test lo fija con un archivo de la forma vieja.
