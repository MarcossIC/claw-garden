# agent-eval-sessions — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (1, 5): si clawg hace rendir más una sesión larga, un relevo entre sesiones y un store grande | M–L, sólo en `crates/clawg-eval` | [agent-eval](../agent-eval/proposal.md) (96), del que toma el arnés; sigue a [memory-delivery](../archive/2026-11-13-memory-delivery/verify.md) (127, archivado); deja el arnés que usan repeated-errors (131) y protocol-block (132) | exploración hecha el 2026-11-13; sigue en la [propuesta](proposal.md) |

**Tesis.** La campaña de memory-delivery no dio ganador porque medía sesiones que no existen: de 2 a 7 turnos y unos 17.000 tokens ([verify](../archive/2026-11-13-memory-delivery/verify.md#lo-que-la-campaña-no-podía-medir)). Una sesión real de este proyecto tiene de mediana 40 pedidos y 62 llamadas, empieza en unos 70.000 tokens y pasa de 200.000 en la mitad de los casos. Lo que una memoria ahorra o cuesta está ahí: en el relevo entre una sesión y la siguiente, en un store con cientos de memorias y en memorias que escribió una sesión anterior, no en una sembrada a mano. El arnés de hoy corre una sola sesión por corrida y no puede medir nada de eso. Este cambio le da sesiones encadenadas, una bifurcación que juega la segunda parte de las dos formas, un ruido de 300 reglas y el costo de cada llamada, y fija antes de correr qué resultado decide cada hipótesis.

## Lo que se midió sobre sesiones reales

Dos pasadas de sólo lectura sobre los transcripts de Claude Code de este proyecto, del 2026-11-11 y el 2026-11-13. Los scripts y sus salidas quedan fuera de git.

| Dato | Valor |
|---|---|
| Sesiones con herramientas | 281 (277 en la primera pasada) |
| Pedidos y llamadas por sesión, mediana | 40 y 62 (p90 119 y 175) |
| Primer pedido, mediana | 69.841 tokens (72.015 en la segunda pasada) |
| Contexto máximo, mediana | 204.567 tokens (p90 404.245) |
| Sesiones que pasan de 150.000 y de 200.000 tokens | 191 (68 %) y 146 (52 %) |
| Sesiones que buscan en clawg | 199 de 277 |
| Sesiones que empiezan con `/clear` | 155 de 281 |
| Compactaciones | 35, todas automáticas; sólo 8 con trabajo después |
| Pares de sesiones seguidas del mismo día y tema | 124; la segunda relee el 30 % de lo que lee antes de editar, y sólo el 26 % de eso es evitable (media 1.740 tokens por par) |
| Relectura de la segunda sesión de un par, en toda la sesión | mediana 4.800 tokens, media 8.973, p90 24.367 |
| Errores de herramientas evitables, en todos los proyectos de la máquina | 1.292 de 1.724; cinco comandos que no existen en el shell suman 699 intentos |

Tres lecturas que cambian el diseño:

- **El relevo que se usa es `/clear`, no la compactación.** Más de la mitad de las sesiones arranca así. La comparación que importa es una sesión que sigue contra una sesión nueva sobre el mismo directorio, con y sin memoria.
- **Cortar sólo rinde arriba de 200.000 tokens.** A 150.000 el contexto nuevo deja poco margen y el ahorro se va en volver a escribir la caché.
- **La relectura no es lo que limita el relevo.** Resta de 1 a 4 puntos. Lo que los transcripts no dicen es si la sesión nueva resuelve igual: eso sólo lo mide una corrida.

### Cuánto ahorra un corte, según el precio de leer la caché

La simulación sobre las 281 sesiones corta al pasar de 200.000 tokens y arranca de nuevo con el contexto del primer pedido más el sobrecosto del relevo (el resumen más lo que la sesión nueva relee). El ahorro de contexto leído es del 39 al 43 %. El ahorro de costo depende de cuánto cuesta leer la caché frente a la entrada:

| Sobrecosto del relevo (tokens) | Lectura a 0,1× la entrada | A 0,05× | A 0,025× |
|---|---|---|---|
| 0 (techo) | 24,9 % | 15,1 % | 5,3 % |
| 5.000 | 24,0 % | 14,1 % | 4,2 % |
| 10.000 | 23,2 % | 13,2 % | 3,2 % |
| 15.000 | 22,2 % | 12,1 % | 2,0 % |
| 20.000 | 21,1 % | 10,7 % | 0,4 % |
| 25.000 | 20,1 % | 9,6 % | −1,0 % |
| 30.000 | 19,1 % | 8,2 % | −2,7 % |
| 40.000 | 16,7 % | 5,2 % | −6,2 % |

La primera columna es la de la estimación original (del 18 al 23 % del costo) y supone la relación de precios de lista de antes. El modelo de las campañas pagas, Sonnet 5.5, lee la caché a 0,05 veces la entrada: los costos que estimó el cliente en el piloto se reproducen con 0,10 USD por millón de tokens de lectura de caché, 2,50 de escritura a 5 minutos, 2,00 de entrada y 10,00 de salida (la mediana de `a0`: 45.023 leídos, 3.931 escritos y 494 de salida dan 0,0193 USD, lo que el cliente informó). Con esa relación el techo es el 15 %, así que una regla de «ahorro de al menos el 15 % del costo» no la puede cumplir ningún relevo real. Con una relación de 0,025 el corte casi no ahorra. La regla de decisión de la [propuesta](proposal.md#la-regla-de-decisión) se escribe por eso sobre lo que sí mide el arnés, la seguridad del corte y su sobrecosto en tokens, y deja esta tabla para pasar el sobrecosto a ahorro según el modelo que se use.

## Lo que el arnés no puede hoy

Todo en `crates/clawg-eval/src/bin/agent-eval/`, leído en `8a2f3723`.

- **Una corrida es una sesión.** El plan lleva un solo comando de cliente (`RunPlan.client`, `arm.rs:187`), con el prompt de la tarea en `-p` (`arm.rs:295-296`). `execute` lo lanza una vez entre los comandos de antes y los de después (`run.rs:466-490`), y un cliente que no sale con 0 es un error que no deja resultado (`run.rs:492-498`). `start_client` crea `stream.jsonl` y `client.log` de nuevo cada vez (`run.rs:688-701`) y `score_dir` lee ese único stream (`run.rs:718-722`).
- **La base para encadenar ya está.** El directorio de la corrida fija el home, el directorio de trabajo, la configuración del cliente y el store (`Layout`, `arm.rs:98-135`), y `prepare` los deja listos antes de arrancar nada (`run.rs:341-373`). Dos sesiones lanzadas dentro del mismo `execute` comparten daemon, store, memoria nativa y archivos sin tocar esa base.
- **Una tarea tiene un prompt.** `Task.prompt` es un `String` obligatorio (`task.rs:34`) y la estructura rechaza campos desconocidos (`task.rs:25`). `run.json` guarda la tarea entera (`run.rs:357-369`) y `score_dir` la vuelve a leer (`run.rs:710`), así que un cambio de forma tiene que seguir leyendo las corridas ya hechas.
- **Un trazador sólo puede nacer en la siembra.** `problems` rechaza el que no está en ninguna memoria sembrada y el que está en el prompt (`task.rs:176-179`). Un hecho que la persona dice en la sesión 1 y que la sesión 2 necesita no se puede declarar.
- **Los checks no tienen sesión.** `score` los evalúa contra la respuesta final, los archivos y todas las llamadas (`score.rs:35-53`). No hay check que cuente fallos de una herramienta.
- **Ningún brazo tiene shell.** Todos niegan `Bash` (`arm.rs:316-320`), y un nombre de herramienta negado la saca del contexto del agente, según la referencia del cliente: no puede ni intentarla. Un error repetido de un comando que no existe no se puede provocar.
- **Las categorías `D` y `R` existen sin tareas** (`task.rs:11`); las seis tareas reales son K1, K2, K3, S, C y N.
- **Sólo se leen los tokens del primer pedido.** `stream::parse` toma el `usage` del primer mensaje del asistente y nada más (`stream.rs:158-163`); `ToolCall` no guarda tokens (`stream.rs:14-26`). En un stream real cada evento del asistente trae el `id` de su mensaje y su `usage`, así que el dato está y no se lee.
- **El `MEMORY.md` de `a4` va al revés.** `seed::seeded` ordena de la más nueva a la más vieja (`seed.rs:80-84`) y ese orden llega al archivo (`arm.rs:232`, `arm.rs:276-279`, `seed.rs:87-95`). Un agente que agrega notas al final deja la más nueva abajo, y en la tarea C el agente eligió «la que aparece más tarde», que era la vieja: el 0 de 2 de `a4` fue del arnés ([verify](../archive/2026-11-13-memory-delivery/verify.md#lo-que-no-cuenta-como-resultado)).
- **`delivered` da por entregado el `MEMORY.md` entero** (`run.rs:185-193`), el que escribió el plan. No mira lo que el agente escribió después ni lo que el cliente carga de verdad.
- **El ruido es uno.** `load_noise` lee `noise.json` (`task.rs:82`, `task.rs:212-234`), que tiene 15 reglas. No tiene tope, y los tests lo cuentan del archivo (`tests/agent_eval.rs:19-22`, `:164`), así que 300 reglas no rompen nada: falta poder elegir el ruido de una corrida.
- **El reporte agrupa por tarea y brazo** (`report.rs:119-164`) y los costos van una línea por sesión y modelo (`report.rs:206-261`), con una sesión por corrida.
- **La corrida en seco no llama a clawg.** `replay-client` aplica los `Write` y `Edit` del stream y lo imprime (`replay.rs:30-50`, `:82-97`). Sirve para probar la plomería, no la memoria: lo que la sesión 1 «guardó» no está cuando la sesión 2 «busca».

## Lo que documenta el cliente

Leído el 2026-11-13 en https://code.claude.com/docs/en/memory, https://code.claude.com/docs/en/cli-reference y https://code.claude.com/docs/en/hooks. Como en el resto del arnés, cada cosa se confirma con la primera sesión real.

| Hecho | Consecuencia |
|---|---|
| De `MEMORY.md` se cargan al arrancar las primeras 200 líneas o los primeros 25 KB, lo que llegue antes; lo demás no entra. Los archivos de tema no se cargan | Con 300 reglas el archivo de `a4` tiene unas 900 líneas (`seed.rs:87-95` escribe una línea en blanco y el texto de cada memoria, de dos líneas en el ruido). En el orden en que un agente lo deja, entran las 66 más viejas |
| La salida de un hook (`additionalContext` incluido) tiene un tope de 10.000 caracteres, sin ajuste; lo que pasa se guarda en un archivo y el agente ve la ruta y 2.000 caracteres | `l1` no puede entregar 300 reglas por su hook: son unos 45.000 caracteres. El producto ya cuenta con ese tope (`crates/clawg-cli/src/hook.rs:666`) |
| `claude -p --continue` sigue la conversación más reciente del directorio, incluidas las de `-p` | Una sesión larga se puede jugar como varios procesos, cada uno con su prompt, sin cambiar la tarea |
| `SessionStart` corre otra vez al continuar, con `source` `resume` | En la forma continua el hook de clawg corre también en la segunda parte; se informa, no se corrige |
| `--disallowedTools` con un nombre quita la herramienta; una regla con alcance, como `Bash(rm *)`, la deja y niega sólo esas llamadas | Un shell acotado se arma permitiendo comandos nombrados, no negando |
| `--max-budget-usd` no cuenta lo gastado antes de continuar | El tope de gasto es por proceso, no por corrida |

## Enfoques

**Cómo se juega «una sesión larga».**

| Enfoque | A favor | En contra |
|---|---|---|
| Un prompt con las dos partes | Sin cambios en el corredor | El agente ve la parte 2 desde el principio: es otra tarea |
| Un proceso con `--input-format stream-json` y los prompts por stdin | Una conversación de verdad | El corredor tiene que hablar el protocolo, esperar cada resultado, y la corrida en seco no lo reproduce |
| Un proceso por parte, el segundo con `--continue` | El mismo bucle sirve para las dos formas; cada parte deja su stream y su costo; la conversación y su caché son las mismas | `SessionStart` corre de nuevo; hay que comprobar que continuó |

Se recomienda el tercero.

**Cómo se compara con el relevo.** Correr por separado una corrida continua y una partida paga dos veces la primera parte, que es la cara (llevar el contexto arriba de 150.000 tokens es escribir esa caché), y compara dos primeras partes distintas. Bifurcar la corrida después de la primera sesión (guardar el estado, jugar la segunda parte continuando, restaurar, jugarla en una sesión nueva) paga la primera parte una vez y deja pares: la diferencia es sólo la segunda parte. Cuesta más código en el corredor y exige restaurar bien el directorio de trabajo, el store y la configuración del cliente. Copiar la corrida a otra ruta no sirve: el proyecto de clawg y la memoria nativa se identifican por la ruta del directorio de trabajo.

**La memoria nativa con 300 reglas.** No tiene un estado documentado que las conserve todas: el cliente le pide al agente que acorte el índice cuando se pasa. Sembrar un índice ya resumido sería inventar lo que el agente habría hecho. Se siembra el archivo como lo deja un agente que agrega al final, se modela la carga documentada y un canario pago la comprueba con los tokens del primer pedido. El resultado de `a4` a esa escala es en buena parte por construcción, y la propuesta lo dice.

**Un comando que no existe.** Tres formas: un servidor MCP del arnés con una herramienta que siempre falla (hay que escribir un servidor, y `a0` dejaría de estar sin servidores), un hook `PreToolUse` que niega una herramienta nativa con un mensaje (no es un fallo de la herramienta: un hook del producto que mire fallos no lo vería), o un shell acotado a comandos nombrados, uno de los cuales no está. La tercera es el caso real, el que mide 131, y la que se recomienda.

## Preguntas que quedan para la primera sesión real

- Si `claude -p --continue` conserva el contexto en `-p` con un `CLAUDE_CONFIG_DIR` propio. Lo dice el primer pedido de la segunda parte.
- Si la memoria nativa escribe en `-p`. Si no escribe, `a4` partida es `a0` partida, y eso es un dato del modo, no de la memoria.
- Si el cliente carga 200 líneas del `MEMORY.md` o el archivo entero. Lo dice el canario.
- Cómo llega al stream un comando de shell que no existe, y un comando no permitido.
- Si la primera parte de la tarea de relevo llega a 150.000 tokens.

## Fuentes

- El [verify](../archive/2026-11-13-memory-delivery/verify.md) de memory-delivery y los resultados de su campaña y del piloto.
- Las dos pasadas sobre los transcripts, del 2026-11-11 y el 2026-11-13, fuera de git; la tabla del ahorro sale de la simulación de la segunda, corrida de nuevo con las tres relaciones de precio.
- Las páginas del cliente citadas arriba, leídas el 2026-11-13.
- Código de `develop` en `8a2f3723`, con las rutas y líneas citadas en cada punto.
