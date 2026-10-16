# redaction-precision — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 (y transversal) | S | — (tiene que cerrar antes de 44; se coordina con 36 y 22; ver «Interacciones») | exploración (borrador) |

**Tesis:** el enmascarado corre antes de trocear, hashear y embeber. Si enmascara de más, el texto original no llega nunca a la base, así que un falso positivo destruye texto del usuario sin vuelta atrás. Hoy hay al menos un falso positivo verificado: un identificador con guiones que empieza con `sk-` y lleva un dígito se enmascara como clave de OpenAI.

En el otro sentido, lo que el agente o el usuario **saben** que es secreto no tiene cómo marcarse, y los secretos sin un formato conocido (una contraseña en `password=`, un `Authorization: Bearer`) pasan enteros.

Precisión primero, y después una marca explícita: las dos cosas protegen la base del usuario.

## Premisas que se verificaron

| Premisa | Resultado |
|---|---|
| El enmascarado corre antes que todo lo demás | **Cierta.** `let masked = redact(&evt.content)` en el worker, antes del chunker, del hash de dedup, del embedding y del FTS (`crates/clawg-core/src/memory/worker.rs:427-430`). El índice de docs también enmascara antes de trocear (`crates/clawg-core/src/memory/project_docs/mod.rs:378`). |
| Los patrones son por formato y se guardan con `\b` | **Cierta.** Hay diez patrones en `PATTERNS` (`crates/clawg-core/src/memory/redact.rs:63-82`). Todos llevan `\b`, salvo la clave privada (`:72-75`). Después, `accepts()` filtra (`:86-100`). Para OpenAI exige que lo capturado no empiece con `ant-` y que tenga al menos un dígito y una letra (`:90-94`). |
| Una palabra que termina en `sk-` no dispara el patrón | **Cierta.** En `task-runner-…` o `risk-assessment-…`, la `s` va precedida de una letra, así que `\b` no se cumple. Se comprobó con `rg`, que usa el mismo motor que el crate `regex`, sobre el patrón copiado de `redact.rs:68`. |
| Un identificador con guiones que **empieza** con `sk-` y lleva un dígito se enmascara | **Cierta, falso positivo verificado.** `the sk-learn-pipeline-for-text-classification-v2-2026-model` coincide con `\bsk-(?:proj-|svcacct-|admin-)?([A-Za-z0-9_-]{20,})` (`redact.rs:68`): `sk-` va después de un espacio, sigue una tira de 20 o más caracteres válidos y tiene dígitos y letras, así que pasa `accepts()`. Lo mismo pasa con una ruta como `scripts/sk-…`. |
| Ese caso tiene test | **No.** `ordinary_identifiers_and_hashes_are_left_alone` (`redact.rs:201-214`) cubre `sk-learn-is-not-a-key-at-all`, que pasa sólo porque no tiene dígitos. |
| Hay una marca explícita para lo privado | **No existe.** Un `rg` de `<private` y `private>` en `crates/` no encuentra nada. El mapa de `openspec/README.md` ya la anotó como complemento útil, con la regla de que una marca sin cerrar enmascara hasta el final («Mejoras propuestas»). |
| Hay detección por contexto o por entropía | **No.** `redact.rs:4-6` lo dice: «Nothing here guesses from entropy». El único patrón con contexto es la contraseña dentro de una URL (`:77`), y descarta los marcadores de posición que empiezan con `$ < { [ *` (`:97`). |
| Una clave privada sin su línea final se enmascara hasta el final del texto | **Cierta, y es a propósito.** La alternativa `.*` de `redact.rs:72-75` y el test `a_private_key_without_its_end_line_is_masked_to_the_end`. Consecuencia: un save que explica el formato PEM y nombra la cabecera pierde todo lo que sigue. |
| `clawg secrets` y `purge` miran sólo `memory` | **Cierta.** `scan` hace `SELECT … FROM memory` (`crates/clawg-core/src/memory/secrets.rs:41`) y clasifica con `redact` (`:46`). Las tablas del índice de docs se derivan del disco y ya entran enmascaradas. |

## Problema (verificado contra el código)

- **Falso positivo = pérdida.** El texto enmascarado es el que se trocea, se hashea y se embebe. El original no queda en ninguna tabla ni en ningún snapshot. Un `[REDACTED:openai_key]` en lugar del nombre de un pipeline, un paquete o un script no tiene arreglo después.
- **Dónde aparece el patrón.** `sk-` es un prefijo común: scikit-learn (`sk-learn`), nombres de scripts, ramas y paquetes. Con un dígito en el nombre (una versión, un año) y 20 caracteres o más, el filtro de `accepts()` no lo frena.
- **Falso negativo = el secreto queda para siempre.** Una contraseña escrita como `password=…`, un `Authorization: Bearer …` o una clave de un proveedor sin patrón entran en claro. Hasta un `purge` viajan a cada snapshot, y con el sharing (objetivo 4) a cada agente que lee.
- **Lo que el agente sabe y la regex no.** Si el usuario pega un dato sensible, el agente puede saber que lo es. No tiene cómo decir «esto no se guarda».

## Lo que ya existe y se reutiliza

- `redact` es pura y su segunda pasada no encuentra nada nuevo (`redact.rs:131`; test `masking_twice_finds_nothing_new`). Cualquier regla nueva se prueba con una tabla de casos, sin base.
- `accepts()` ya es el lugar para filtros posteriores al match (`redact.rs:86-100`), así que no hace falta lookbehind ni una dependencia de regex con más funciones.
- El rechazo de marcadores de posición de las URL (`redact.rs:97`) sirve igual para `password=<tu-clave>`.
- `clawg secrets` clasifica con la misma función (`secrets.rs:46`): una regla nueva se ve sobre las filas ya guardadas sin código extra.

## Enfoques posibles

### A. Precisión del patrón de OpenAI (recomendado)

Un filtro puro en `accepts()` que rechace lo que no parece aleatorio, por ejemplo cuando lo capturado son varias palabras en minúscula separadas por guiones. Las claves reales no tienen esa forma, y un identificador sí.

- **A favor:** cierra el falso positivo sin tocar los otros nueve patrones.
- **En contra:** toda regla que rechaza puede dejar pasar una clave real. Por eso el criterio sale de una tabla de casos con claves de formato válido (sintéticas) y nombres reales de identificadores, y se fija con tests en los dos sentidos.

### B. Marca `<private>…</private>` (recomendado)

Lo marcado se reemplaza por `[REDACTED:private]` en todo lugar donde corre `redact`: el save y el índice de docs. Una marca sin cerrar enmascara hasta el final del texto, que es la regla ya anotada en el mapa.

- **A favor:** es barato, lo controla quien escribe, y cubre lo que ninguna regex puede conocer.
- **En contra:** depende de que el agente lo use. memory-protocol (22) tiene que decirlo.

### C. Patrones por contexto

El valor después de `password=`, `passwd:`, `secret=`, `token=`, `api_key=` o `Authorization: Bearer`, cuando es largo y no es un marcador de posición.

- **A favor:** atrapa secretos sin formato conocido.
- **En contra:** riesgo de falsos positivos en documentación, código de ejemplo o configuraciones con valores de prueba. Los marcadores de posición se descartan como en las URL. Lo que queda se decide con la tabla de casos.

### D. Más formatos conocidos

Proveedores que hoy no tienen patrón. Cada uno entra sólo con su formato documentado y sus casos de test, nunca por intuición.

## Descartado

| Opción | Razón técnica |
|---|---|
| Detección por entropía sin contexto | Enmascara hashes de commit, UUID y sha256, que hoy se dejan a propósito (`ordinary_identifiers_and_hashes_are_left_alone`). En una base de memorias de código, eso es pérdida segura. |
| Guardar el original cifrado para poder revertir un falso positivo | Vuelve a guardar el secreto, que es lo que el enmascarado evita. El cifrado en reposo ya se descartó en el mapa por el riesgo de perder la clave. |
| Una dependencia de regex con lookbehind | `\b` y `accepts()` ya cubren el borde izquierdo. Lo que falla es el criterio sobre el resto del match, no el borde. |
| Avisar en lugar de enmascarar | Un secreto que entra en claro va a los snapshots y a otros agentes antes de que alguien lea el aviso. |

## Interacciones

- **history-import (44).** Miles de textos pasan por `redact` en una sola importación. Un falso positivo ahí se multiplica y no tiene vuelta atrás. Este cambio va antes.
- **ingest-outbox (36).** Mueve el enmascarado al handler para la copia que persiste. Es la misma función, y las reglas nuevas aplican sin cambios.
- **memory-protocol (22).** El texto del protocolo dice cuándo usar `<private>`.
- **Índice de docs.** El índice se re-lee cuando cambia el sha256 de un archivo o el modelo (`.claude/CLAUDE.md`), no cuando cambian las reglas de `redact`. Un pasaje indexado con la regla vieja queda así hasta que el archivo cambie. Decisión: forzar un reindexado al subir la versión de las reglas, o aceptarlo, porque el índice es derivado y no destruye nada del usuario.
- **`clawg secrets`.** Con las reglas nuevas lista también las filas guardadas antes que tengan un patrón nuevo. Es la forma de encontrarlas y decidir un `purge`.

## Alcance candidato

1. Una tabla de casos versionada en los tests de `redact.rs`: claves sintéticas de cada formato, que se enmascaran, e identificadores, rutas, hashes y marcadores de posición, que no.
2. El filtro de A en `accepts()` para OpenAI.
3. La marca `<private>` en `redact`, con su regla de marca sin cerrar.
4. Los patrones de contexto (C) y los formatos nuevos (D) que se acepten, cada uno con sus casos.
5. Docs: `.claude/CLAUDE.md` (qué se enmascara) y `specs/secrets`.

### No-alcance

- Reescribir filas ya guardadas. Lo hace `purge`, a pedido.
- Entropía sin contexto.
- Cambiar la regla de la clave privada sin línea final, salvo que se decida (ver decisiones).

## Decisiones abiertas

Son del usuario.

- [ ] **Criterio de A.** ¿Qué forma rechaza el filtro (palabras en minúscula separadas por guiones, proporción de guiones, ausencia de mayúsculas)? Se elige con la tabla de casos.
- [ ] **Marca.** ¿`<private>` como nombre? ¿Se enmascara también en el índice de docs (recomendado: sí, la función es una sola)?
- [ ] **Contexto (C).** ¿Qué claves entran, y desde qué largo cuenta un valor?
- [ ] **Formatos nuevos (D).** ¿Cuáles?
- [ ] **Clave privada sin línea final.** ¿Se mantiene enmascarar hasta el final (seguro, pierde texto en una explicación del formato) o se limita a un largo máximo?
- [ ] **Reindexado de docs** al cambiar las reglas.

## Cómo se mide

Tests RED primero:

1. `the sk-learn-pipeline-for-text-classification-v2-2026-model` queda sin tocar. Hoy se enmascara.
2. Toda clave sintética con formato válido sigue enmascarada (sin regresión), con un caso por formato.
3. `<private>x</private>` queda `[REDACTED:private]`. Una marca sin cerrar enmascara hasta el final. Dos marcas, dos reemplazos.
4. Los patrones de contexto que entren enmascaran el valor y no la clave. `password=<tu-clave>` no se toca.
5. `masking_twice_finds_nothing_new` sigue valiendo con las reglas nuevas.
6. `clawg secrets` lista una fila guardada antes con un patrón nuevo.

`rag-eval` no aplica: no cambia el ranking. Una corrida del corpus del golden set por `redact` debería dar cero cambios. Un cambio ahí es un falso positivo nuevo que hay que mirar.

## Riesgos

- **Cambiar falsos positivos por falsos negativos.** El filtro de A puede dejar pasar una clave con forma rara. Los tests en los dos sentidos son la red, y el criterio se elige mirando la tabla, no a ojo.
- **Una marca que nadie usa.** Sin el texto de 22, B no cambia nada.
- **Textos ya perdidos.** Los falsos positivos de antes de este cambio no se recuperan. Lo único posible es dejar de producirlos.
