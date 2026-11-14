# agent-eval-sessions — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (1, 5) | M–L, sólo en `crates/clawg-eval` (el binario `agent-eval` y sus datos) | [agent-eval](../agent-eval/proposal.md) (96); sigue a [memory-delivery](../archive/2026-11-13-memory-delivery/verify.md) (127, archivado) | tareas escritas el 2026-11-13; sin aplicar |

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
| `requests-timeout-decision-relay` | R | `psf-requests` | 2. La primera pide un inventario de cómo se manejan los timeouts, con archivo y línea, sobre todos los módulos de `src/requests/`, los tests, `docs/user/advanced.rst`, `docs/user/quickstart.rst` y `HISTORY.md`, escrito en `TIMEOUTS.md`; la persona dice ahí la decisión que se aplica después y pide no tocar el código todavía. La segunda dice que la revisión aprobó y pide aplicarla, documentarla y anotarla en `HISTORY.md` | Nacen en la sesión 1: `FORK_DEFAULT_TIMEOUT` (el nombre de la constante) y `4.5, 41` (su valor) | `src/requests/adapters.py` contiene los dos; `docs/user/advanced.rst` contiene `4.5, 41`; `HISTORY.md` contiene `FORK_DEFAULT_TIMEOUT`; `TIMEOUTS.md` contiene `adapters.py` |
| `requests-redact-decision-revised` | D | `psf-requests` | 3. La primera pide leer `src/requests/utils.py` y dice la decisión para una función que se agrega después. La segunda la revisa, sin repetir la anterior, y pide un test sin relación. La tercera pide implementar `redact_url_credentials` «como quedó decidido» | Sesión 1: `[hidden]` (la máscara de la primera decisión). Sesión 2: `_:_` (la de la revisada) | `src/requests/utils.py` contiene `def redact_url_credentials` y `_:_`, y no contiene `[hidden]`; `tests/test_utils.py` contiene `redact_url_credentials` |
| `requests-history-owner-convention` | K2 | `psf-requests` | 1, de 12 a 15 pedidos: agregar `to_curl()` a `PreparedRequest`, documentarlo y anotarlo en `HISTORY.md` «con las convenciones de este fork» | Sembrado, 40 días, regla de `HISTORY.md`: cada entrada del fork termina con `(owner: resilience-guild)`. Trazador `resilience-guild` | `src/requests/models.py` contiene `def to_curl`; `HISTORY.md` contiene `resilience-guild` |
| `shelf-eza-missing` | E | `shelf-notes` | 3, con el shell acotado a `eza` y `ls`. El `CLAUDE.md` del fixture manda listar con `eza`, que no existe. Las dos primeras piden contar los archivos de `src/`; la tercera, los de `docs/` | Ninguno: se mide una conducta | En la sesión 3, la respuesta tiene el conteo y `tool_failed_count` de `Bash` con `eza: command not found` es 0 |

Lo que se toma de la fuente y lo que se escribe acá va en `data/agent-tasks/SOURCES.md`: de `psf-requests` sólo el fixture; los prompts, los hechos, las memorias, los checks y las edades son de acá. Las memorias de las seis tareas del piloto no cambian.

Dos tareas de ejemplo, sobre `example-tally`, prueban el arnés en CI: una de relevo de dos sesiones y una con un comando ausente.

## La regla de decisión

Se fija acá, antes de correr, y se lee con estas reglas:

1. **Sin «casi».** Un resultado que no cumple una condición no gana. No se agregan corridas para desempatar.
2. **Corridas por celda fijas:** las de la tabla del [presupuesto](#presupuesto). Una celda es una tarea bajo un brazo, una forma y un ruido.
3. **Artefactos.** Una corrida se excluye, y se repite una sola vez, sólo por una de estas causas, que el resultado muestra sin interpretación: (a) una sesión sin resultado o que terminó en error; (b) en un brazo con clawg, el servidor no figura `connected` en alguna sesión; (c) en la forma continua, el primer pedido de la segunda sesión tiene menos contexto que el último de la primera (no continuó); (d) un comando que la tarea declara ausente estaba. Una continuación que arrancó con la caché fría (menos del 80 % de su primer pedido leído de caché) cuenta para el éxito y no para el costo.
4. **Si se excluyen más de 2 corridas en una fase, la fase se detiene** y se revisa el arnés antes de gastar más.
5. **Lo que es por construcción se dice.** Un resultado que el diseño de la corrida ya determina no se presenta como hallazgo.

### H1, el relevo

Se lee por brazo X, con `a3` y `a4` en esta campaña (`l0` lo corre 132 sobre la misma tarea). En R, cada corrida es un par: la misma primera sesión, y la segunda jugada continua y nueva.

- **Validez:** en R, la forma continua aprueba en al menos 2 de los 3 pares de X, y el contexto al bifurcar es de 150.000 tokens o más de mediana; con menos contexto la regla se aplica igual y el resultado se informa como «régimen de N tokens». En D, la forma continua bajo `a0` aprueba 2 de 2. Una tarea que no es válida no decide: se informa como rota y H1 queda sin veredicto para ese brazo.
- **Seguro:** en R, ningún par en que la forma continua aprueba y la nueva falla; y en D, la forma partida aprueba 3 de 3.
- **Sobrecosto:** la mediana, en los pares de R, de los tokens que el relevo le suma a la sesión nueva: lo que su primer pedido tiene de más sobre el primer pedido de la sesión 1, más lo que relee (los `Read` de archivos que la sesión 1 ya leyó y las lecturas de memoria, a 4 bytes por token).
- **Veredicto de X:** `rinde` si es seguro y su sobrecosto es de 20.000 tokens o menos; `seguro pero caro` si es seguro y pasa de 20.000; `no es seguro` en otro caso.

El umbral de 20.000 sale de la [tabla de la exploración](exploration.md#cuánto-ahorra-un-corte-según-el-precio-de-leer-la-caché): hasta ahí un corte a 200.000 tokens ahorra el 10,7 % del costo con un modelo que lee la caché a 0,05 veces la entrada (Sonnet 5.5 y Opus 5.5) y el 21,1 % a 0,1 veces. Con una relación de 0,025 el corte no ahorra, sea cual sea el veredicto, y el informe lo dice junto al veredicto.

El ahorro medido en cada par (1 menos el costo de la corrida nueva sobre el de la continua, en USD del cliente) se informa y no decide: depende de cuánto dura la segunda parte de esta tarea, y la regla original de «15 % o más» está por encima del techo a los precios del modelo de la campaña.

`a0` partida es el `/clear` sin nada. Si `a0` también es seguro y su sobrecosto no supera al de X, la memoria no aportó al relevo en estas tareas: se informa así y no se construye nada sobre este resultado.

Qué sigue: si `a3` rinde y `a0` no es seguro, se propone el aviso de corte y el resumen de relevo sobre lo que clawg ya tiene. Si `a3` no es seguro y `a4` sí, el cambio siguiente se diseña contra lo que hace la nativa. Si ninguno es seguro, el relevo necesita un resumen escrito antes del corte, y eso es una hipótesis nueva, no otra lectura de ésta.

### H2, la escala

Sobre `requests-history-owner-convention`, 3 corridas por celda, con 15 y con 300 reglas:

- **No medible:** si `a3` con 15 reglas aprueba menos de 2 de 3. El agente no tira de la memoria ni con el store chico, y la escala no es lo que limita.
- **clawg aguanta:** `a3` con 300 aprueba al menos tantas como con 15; en S, C y N del piloto con 300, `a3` y `l0` aprueban 2 de 2; y la mediana del primer pedido de `a3` con 300 no supera en más de 500 tokens a la de 15.
- **Ventaja en escala:** clawg aguanta y `a3` con 300 aprueba al menos 2 corridas más que `a4` con 300.

Lo que es por construcción: el `MEMORY.md` de `a4` con 300 reglas tiene unas 900 líneas, el cliente carga 200 y la memoria de la tarea, de 40 días, queda fuera. Que `a4` no la reciba al arrancar no es un hallazgo; lo que se mide es si el agente va a buscarla al archivo. Si el canario muestra que el cliente carga el archivo entero, el modelo de carga se corrige antes de la fase y esta salvedad cae.

### H3, el ciclo de vida

No tiene ganador propio: es la condición de seguridad de H1 con su causa. Cada hecho nacido en una sesión se clasifica en la última, por el resultado y sin juicio: `no guardado` (ninguna sesión anterior lo escribió en una herramienta de clawg ni en un archivo de la memoria nativa), `guardado y no llegó`, `llegó y no se usó`, `usado`. En D, además: `usó la primera` si el archivo tiene `[hidden]`. Un brazo **mantiene** un hecho si sale `usado` en 3 de 3.

### H4, el orden del `MEMORY.md`

C bajo `a4`, con 15 reglas y el orden corregido, 2 corridas. Se informa junto al 2 de 2 del piloto y al 0 de 2 de la campaña. No decide nada.

### H5, el costo por llamada

Por herramienta de clawg: llamadas, cuántas se pudieron medir solas, y la mediana y el p90 de los tokens que agregan. Una herramienta con mediana de más de 1.500 tokens, el doble de lo que `search_memory` mide en agent-view (`openspec/config.yaml:28`), queda anotada como candidata a una dieta. No decide nada más.

### H6, el error repetido

Sobre `shelf-eza-missing`, 3 corridas por brazo:

- **Hay efecto:** bajo `a0`, la sesión 3 falla con `eza` en al menos 2 de 3.
- **La memoria de hoy ya lo evita**, para `a3` o `a4`: 0 fallos en la sesión 3 en 3 de 3.

Si hay efecto y ninguna memoria lo evita, 131 tiene su línea base y su meta, 0 fallos en 3 de 3. Si no hay efecto, la tarea no mide lo que 131 necesita y 131 la cambia en su propuesta.

## Presupuesto

Con Sonnet 5.5 y los precios de la exploración. Una sesión corta costó de 0,023 a 0,038 USD de media en la campaña anterior. Una larga se estima por sus tokens: escribir en caché lo que entra al contexto, leerlo en cada pedido siguiente y unos 500 tokens de salida por pedido.

| Corrida | Pedidos | Contexto | Estimado (USD) |
|---|---|---|---|
| Tarea del piloto | 2 a 7 | 17.000 a 21.000 | 0,03 |
| `shelf-eza-missing`, 3 sesiones cortas | 9 a 12 | 17.000 a 21.000 | 0,12 |
| `requests-history-owner-convention` | 12 a 15 | hasta 60.000 | 0,30 |
| `requests-redact-decision-revised`, 3 sesiones | 16 a 20 | hasta 60.000 | 0,40 |
| `requests-timeout-decision-relay`, bifurcada | 10 a 12 la primera parte; 10 a 12 cada segunda | 150.000 a 200.000 al bifurcar | 1,25 (0,65 la primera parte, 0,35 la continua, 0,25 la nueva) |

| Fase | Corridas | USD |
|---|---|---|
| 0. Ensayo sin pago: todas las tareas en seco, con el modelo falso en CI y con el real en local | — | 0 |
| 1. Humo y canarios: R bifurcada bajo `a3`; D partida bajo `a3`; la K larga bajo `a3` con 300; E bajo `a0`; N bajo `a0` y bajo `a4` con 300 | 6 | 2,13 |
| 2. Relevo y ciclo de vida: R bifurcada bajo `a0`, `a3` y `a4`, 3 cada una; D continua bajo `a0`, 2, y partida bajo `a0`, `a3` y `a4`, 3 cada una | 20 | 15,65 |
| 3. Escala: las seis del piloto con 300 bajo `a3`, `l0` y `a4`, 2 cada una; las seis bajo `a4` con 15 y el orden corregido, 2 cada una; la K larga bajo `a3` y `a4` con 15 y con 300, 3 cada una | 60 | 5,04 |
| 4. Error repetido: E bajo `a0`, `a3` y `a4`, 3 cada una | 9 | 1,08 |
| **Total** | **95** | **23,90** |

- **Tope: 25 USD.** Cada sesión lleva además su `--max-budget-usd`: 1,20 la primera parte de R, 0,80 cada segunda, 0,50 las de D y la K larga, 0,15 las cortas.
- Las corridas del humo cuentan en su celda si después del humo no cambia ni la tarea ni el arnés; entonces el total baja a unos 21,80.
- **El humo es una puerta.** La campaña no sigue si la primera parte de R cuesta más de 1,00 USD, si su segunda parte no continuó, o si el canario no cuadra con el modelo de carga (el primer pedido de `a4` con 300 reglas supera al de `a0` en más de 8.000 tokens: el cliente cargó el archivo entero).
- **Orden de recorte**, si el costo medio de una fase pasa del estimado en más de la mitad: la K larga baja a 2 corridas por celda; después D continua baja a 1; después las del piloto con 300 corren sólo bajo `a3` y `a4`. R no se recorta.
- La fase 3 es la que menos decide, porque parte de su resultado es por construcción: sin ella el total es 18,86.

## Cómo se ensaya sin pagar

`agent-eval run … --replay DIR` juega cada sesión con un stream escrito a mano. El cliente reproducido aplica sus `Write` y `Edit`, y desde este cambio manda sus llamadas a clawg al servidor de la corrida, por el mismo puente que usa el agente, e imprime lo que el servidor contestó. Así la sesión 2 «busca» de verdad lo que la sesión 1 «guardó»: el store, la siembra, el puente, los archivos por sesión, la bifurcación y el puntaje se prueban enteros sin un modelo. Lo que no se ensaya es si un agente guarda, busca u obedece, que es lo único que pide una sesión paga, ni los hooks del cliente, que la corrida en seco no corre.

## Lugar para 131 y 132

- **131 (repeated-errors)** necesita varias sesiones sobre un mismo home, un shell acotado y un check de fallos: son `sessions`, `shell` con `missing_commands`, y `tool_failed_count`. Su brazo con el hook nuevo es una variante más de `Arm`; nada del modelo de sesiones depende de la lista de brazos. `shelf-eza-missing` es su línea base, y 131 la puede cambiar en su propuesta.
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
| El costo de una sesión larga está estimado, no medido | El humo mide una de cada tipo, y hay tope por sesión, tope total y orden de recorte |

## Datos guardados

Ninguno. El arnés escribe sólo en el directorio de cada corrida. `run.json` y `result.json` ganan campos; los de corridas anteriores se siguen leyendo, y un test lo fija con un archivo de la forma vieja.
