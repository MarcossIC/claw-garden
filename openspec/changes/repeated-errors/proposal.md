# repeated-errors — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 5 (clawg aprende de lo que pasa en la sesión sin que el agente decida guardar) | M: siete commits del producto (el último de docs), una verificación local y uno del arnés | — para el producto; [agent-eval-sessions](../agent-eval-sessions/proposal.md) (129) para la campaña paga | tareas escritas |

## Problema

La [exploración](exploration.md) resume el análisis A2. El 75 % de los errores de herramientas de 578 sesiones se repite después de la 2.ª sesión de su clase. El grupo más grande son los programas que no existen en el Bash de la máquina: `fd` 250 intentos, `eza` 167, `sd` 124, `bat` 104, en hasta 18 proyectos y a unos 26 mil tokens-equivalentes cada uno. El 52 % de lo evitable tiene una firma exacta, `X: command not found`, que un hook ve sin que el agente juzgue nada.

Hoy clawg no ve nada de esto. Ningún hook instalado mira una herramienta (`crates/clawg-cli/src/setup.rs:51-52`) y `HookInput` no lee `tool_name`, `tool_response` ni `error` (`crates/clawg-cli/src/hook.rs:70-92`). Lo que el agente sí guardó a mano no llegó a la sesión siguiente: las sesiones con hook fallan igual que las sesiones sin él. Encima, 408 de los 721 `command not found` terminan con código 0, así que un hook que sólo mira los fallos ve el 43 %.

## Alcance

B, A y D, en ese orden. C queda afuera.

1. **B, el aviso en el mismo turno.** `clawg hook post-tool-use` y `clawg hook post-tool-use-failure`, instalados con matcher `Bash`. Cuando la salida de la llamada trae la firma de un programa ausente que el comando llamó, el hook devuelve una línea por `additionalContext`, en el idioma preferido: el programa no existe en el Bash de esta máquina, no lo llames otra vez en esta sesión, usa Glob, Grep, Read o Edit u otro programa instalado.
2. **A, el libro y la regla al arrancar.** Los mismos hooks anotan en `<data>/tool-failures.jsonl` cada fallo con firma, y el primer éxito verificado después de un fallo. Sólo anotan el programa, la herramienta, la sesión y la hora. `session-start` arma con el libro las reglas que valen y las imprime como la palabra de clawg, junto a la línea del idioma y antes de la evidencia. Una regla vale si en los últimos 30 días el programa falló en dos sesiones distintas o más, no anduvo ninguna vez y no está en el PATH del hook.
3. **D, el diagnóstico.** `clawg doctor` lista las reglas aprendidas y nombra el `CLAUDE.md` que pide cada programa ausente. También avisa cuando el hook de arranque está instalado sin los dos hooks nuevos, con `clawg setup claude-code` como paso.
4. **La medición en agent-eval.** Un brazo `a3-notools`, que es `a3` sin los dos hooks nuevos: es el `a3` que mide 129 antes de este cambio, porque los brazos toman los hooks de `setup::plan`. La tarea `shelf-eza-missing`, el shell acotado y el check `tool_failed_count` son de [agent-eval-sessions](../agent-eval-sessions/proposal.md) (129), que la propone como línea base de este cambio. Se construye cuando 129 esté aplicado.

## Fuera de alcance

- **C**, la guarda en `PreToolUse`. Niega herramientas y corre antes de cada Bash. Se reabre sólo si la campaña muestra que la regla llega al contexto y el agente igual llama al programa.
- **Los rechazos de clawg** (259 evitables, «no están en disputa» de `resolve_conflict`). Se arreglan en esa herramienta y llevan un cambio propio.
- **Los fallos T2 y T3**: sintaxis de shell, `old_string`, parámetros inválidos, rutas inexistentes, tests y builds que fallan, permisos. Su lección no es un hecho de la máquina.
- **La herramienta `PowerShell`, y cmd o PowerShell lanzados desde Bash.** Son el PATH de otro shell.
- **Cursor.** Su `postToolUse` y su `postToolUseFailure` pueden agregar contexto, pero en Windows su shell es PowerShell y su entrada es otra. `--client cursor` con los eventos nuevos no imprime nada ni anota nada.
- **El daemon y `memory.db`.** Este cambio no agrega rutas, no toca `crates/clawg-cli/src/daemon/api.rs` y no cambia el esquema.

## Decisiones

1. **El libro es un archivo del directorio de datos, no una tabla ni memorias.**
   - Una fila `tool_error` por fallo pasaría por la ingesta y despertaría el modelo de embeddings en cada fallo. Competiría en la búsqueda con todo lo demás, necesitaría el daemon vivo en cada llamada y llegaría al agente como evidencia, que es lo que hoy no funciona.
   - Una tabla nueva pediría esquema, migración, una ruta en `api.rs` (que core-use-cases va a partir) y el daemon en cada llamada.
   - El archivo sigue el precedente de `hook-runs.jsonl` (`crates/clawg-cli/src/hook_log.rs:132-141`): una línea por escritura, se renombra a los 64 KiB, nunca crea el directorio de datos y falla en silencio.
   - Es un dato derivado. Borrarlo cuesta sólo volver a aprender, en dos sesiones. No entra en snapshots ni en el export, como el registro de hooks.
   - El alcance es la máquina, por perfil, porque el archivo es del directorio de datos. Es el alcance que A2 mide como el más rendidor (+256 evitables frente al alcance por proyecto).
2. **La regla es la palabra de clawg, no una memoria.** Va en el mismo lugar y con la misma voz que la línea del idioma (`hook.rs:100-112`), antes del bloque que se presenta como «evidencia, no instrucciones». Se imprime aunque el daemon no esté, porque sale de un archivo y no del daemon. Así no se repite el fallo de entrega: no compite por el presupuesto del contexto ni depende de que una búsqueda la traiga.
3. **Una regla vale sólo si nada la contradice.** Pide dos sesiones distintas con fallo en los últimos 30 días, ningún éxito en esa ventana y que el programa no esté en el PATH del hook.
   - El éxito descarta a `rg`: después de cada fallo transitorio queda un éxito.
   - El PATH descarta a `gh` en cuanto se instala, aunque el agente no lo vuelva a intentar.
   - La ventana de 30 días es el TTL.
   - Un éxito sólo cuenta si el comando no redirige stderr (`2>` o `&>`) ni encadena con `||`. Sin esa condición, `eza 2>/dev/null | head` parecería andar.
   - Como mucho se imprimen 5 reglas, de la más reciente a la más vieja.
4. **El aviso del mismo turno no guarda estado.** Sale en cada fallo con firma, salvo que el libro tenga un éxito de ese programa en los últimos 30 días. En ese caso el fallo es probablemente transitorio y el hook no dice nada.
5. **Nada de la llamada se guarda salvo el nombre del programa.** Ni el comando, ni la salida, ni rutas, ni `cwd`. El nombre tiene que tener la forma de un nombre de programa (1 a 32 caracteres de `[A-Za-z0-9._+-]`, empezando por letra o número). Además pasa por el mismo detector de secretos de la ingesta: si el detector encuentra algo, no se anota. Para eso, `redact` pasa de `pub(crate)` a `pub` (`crates/clawg-core/src/memory/domain/redact.rs:235`).
6. **Presupuesto del hook por llamada.**
   - Sin daemon y sin modelo. No lee la preferencia de idioma salvo que haya algo que decir.
   - A lo sumo lee los dos archivos del libro (≤ 64 KiB cada uno) y agrega una línea.
   - Se corta a los 300 ms (`POST_TOOL_DEADLINE`), por debajo de los 5 s con los que se configura.
   - No deja línea en `hook-runs.jsonl`.
   - Puerta en la verificación, en Windows y con el binario publicado: p95 ≤ 60 ms de reloj en 200 corridas desde Git Bash con una entrada sin firma. Hoy `clawg --version` tarda 38 a 43 ms ahí, y el proceso vacío 28 ms. Si no pasa, no se publica con `PostToolUse` instalado.
7. **El texto del arranque entra en el tope del cliente.** El bloque de reglas, de a lo sumo 700 bytes (el peor caso medido son 671), se descuenta del presupuesto que el hook le pide al daemon (`SESSION_BUDGET`, `hook.rs:65`). Así el total sigue debajo de los 10.000 caracteres (`hook.rs:655-667`).
8. **Sin dependencias nuevas.** La firma se reconoce a mano, línea por línea (`: command not found` y las formas de `sh` y `zsh`). Un `regex` en `clawg-cli` costaría su compilación en cada proceso de hook. La búsqueda en el PATH reusa `doctor::analyze` (`crates/clawg-cli/src/doctor/mod.rs:61`).

## Qué cambia para quien ya instaló

`clawg setup claude-code`, con y sin `--local`, instala dos eventos más: `PostToolUse` y `PostToolUseFailure`, los dos con matcher `Bash` y la misma forma exec de los demás. Correrlo otra vez sobre una instalación anterior agrega sólo esos dos y deja los demás byte a byte (R3 de [`specs/setup`](../../specs/setup/spec.md)). `--remove` los saca. `clawg update` no corre `setup`, así que quien ya instaló tiene que correrlo otra vez, y `clawg doctor` lo avisa mientras falten. El servidor MCP, sus herramientas y el bloque del protocolo no cambian, así que `crates/clawg-cli/src/mcp/agent_contract.json` y su versión en español quedan iguales y no se bendicen.

## Cómo se mide el éxito

1. **Sin modelo, en la suite.** Los tests de la [spec](spec.md) fallan hoy. El central alimenta el binario con el JSON de un `eza: command not found` en dos `session_id` distintos, y después `clawg hook session-start` tiene que nombrar `eza`, sin daemon. Las variantes que no tienen que imprimir nada:
   - un solo fallo, o dos en la misma sesión;
   - un fallo seguido de un éxito;
   - `rg` con un fallo por sesión entre éxitos;
   - un fallo de `Edit`;
   - un éxito con `2>/dev/null`, que no borra la regla;
   - el programa presente en el PATH;
   - fallos de hace 31 días.
2. **La puerta de latencia** de la decisión 6, en la verificación.
3. **La campaña** (paga, local, cuando 129 esté aplicado), con la regla de decisión fijada antes de correr:
   - Tarea `shelf-eza-missing` de 129: un `CLAUDE.md` que pide listar con `eza`, que no existe, y el shell acotado a `eza` y `ls`. Las sesiones 1 y 2 piden contar los archivos de `src/`, y la 3 los de `docs/`, sobre el mismo store. Una corrida en la que `eza` estaba se excluye y se repite, como dice 129.
   - Brazos `a3-notools` y `a3`, con 5 corridas cada uno. `a0` sale de la fase 4 de la campaña de 129, porque no corre clawg y este cambio no lo toca.
   - **Antes de aplicar la tarea 4:** si la fase 4 de 129 da que `a3` de hoy ya evita el fallo («la memoria de hoy ya lo evita»: 0 fallos en la sesión 3 en 3 de 3), el bloque del arranque no se construye. Quedan B, el libro y D.
   - **A pasa** si `a3` tiene `tool_failed_count(eza) = 0` en la sesión 3 en al menos 4 de 5 corridas, responde bien al menos tantas veces como `a3-notools`, y el primer pedido de su sesión 3 no supera en más de 300 tokens de media al de `a3-notools`.
   - **B pasa** si, en las sesiones 1 y 2, los fallos de `eza` después del primero de cada sesión de `a3` son a lo sumo la mitad de los de `a3-notools`.
   - **La tarea discrimina** sólo si `a3-notools` falla con `eza` en la sesión 3 en al menos 3 de 5 corridas. Si no, el resultado es «no concluyente» y no cuenta como éxito.
   - **Control N.** En la tarea N del piloto, que no tiene ningún programa ausente, con 2 corridas por brazo: `a3` no recibe ninguna línea de clawg sobre programas ausentes, y la mediana de su costo no sube más de un 5 % frente a `a3-notools`.
   - **Si A no pasa** con la línea presente en el stream, se cambia una vez la redacción de la línea y se mide otra vez. Si tampoco pasa, se saca el bloque del arranque y quedan B y D.

## Riesgos

- **La forma real de `tool_response` de Bash** no está en este repositorio: los tipos del cliente la dejan sin tipo. Por eso el hook busca la firma en todos los textos de `tool_response`, sin suponer campos. La verificación registra una entrada real de cada evento y la suma como fixture.
- **El PATH del hook no es el del Bash del agente.** En Windows, Git Bash suma `/usr/bin`. El PATH sólo se usa para descartar una regla, nunca para crearla, así que el error posible es callar una regla, no inventarla.
- **Un `CLAUDE.md` que pide lo contrario.** La regla no le dice al agente que desobedezca. Dice el hecho, ofrece las herramientas del cliente y pide avisarle a la persona. D le muestra la causa a la persona.
- **El arranque de un proceso en Windows con el antivirus escaneando un binario de 43 MB** puede pasar los 60 ms en frío. Lo muestra la puerta de latencia.
- **Dos hooks que escriben a la vez** (llamadas en paralelo, sesiones en paralelo). Cada línea se agrega con una sola escritura en modo append y el renombrado no relee nada, como en `hook-runs.jsonl`. Perder una línea cuesta, como mucho, una sesión más de aprendizaje.

## Datos guardados

`memory.db` no cambia: ni esquema, ni filas, ni identificadores, así que no hay migración. El único archivo nuevo es `<data>/tool-failures.jsonl`, con `tool-failures.1.jsonl` como versión anterior. `restore`, `purge`, `export` y los snapshots no lo leen ni lo escriben. Vuelta atrás: `clawg setup claude-code --remove` (o la versión anterior) deja de anotar, y borrar los dos archivos olvida todo lo aprendido sin tocar una memoria.
