# redaction-precision — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 (y transversal) | S | — (tiene que cerrar antes de history-import, 44; se coordina con ingest-outbox, 36, y memory-protocol, 22) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código); lo que sigue está verificado otra vez contra `main` (`210c692`).

1. **Un falso positivo destruye texto del usuario.** El worker enmascara antes de trocear, hashear, embeber e indexar (`let masked = redact(&evt.content)`, `crates/clawg-core/src/memory/worker.rs:430`), así que el original no llega a ninguna tabla ni a ningún snapshot. El patrón de OpenAI (`crates/clawg-core/src/memory/redact.rs:68`) más su filtro (`accepts`, `redact.rs:90-94`) aceptan cualquier tira de 20 caracteres o más después de `sk-` que tenga una letra y un dígito: `the sk-learn-pipeline-for-text-classification-v2-2026-model` se guarda como `the [REDACTED:openai_key]`. El test de identificadores (`ordinary_identifiers_and_hashes_are_left_alone`, `redact.rs:201-214`) sólo cubre `sk-learn-is-not-a-key-at-all`, que se salva por no tener dígitos.
2. **Lo que se sabe secreto no tiene cómo marcarse.** No hay ninguna marca: un `rg` de `<private` en `crates/` no encuentra nada. Si el usuario pega un dato sensible, el agente lo guarda tal cual.
3. **Un secreto sin formato conocido entra en claro.** Sólo hay patrones por formato (`PATTERNS`, `redact.rs:63-82`); el único con contexto es la contraseña dentro de una URL (`redact.rs:77`). `password=hunter2`, `DB_PASSWORD=…` o `Authorization: Bearer …` con un token opaco pasan enteros, viajan a cada snapshot y, con el sharing, a cada agente que lee. Tampoco hay patrón para formatos documentados y comunes como los tokens de GitLab o npm.

## Decisiones

Defaults tomados donde la exploración dejaba la pregunta abierta. La de los patrones por contexto se preguntó en el hilo con una tarjeta; mientras Marcos no elija otra, se sigue con la recomendada.

- **Criterio para `sk-`: mayúscula, minúscula y dígito.** Las claves de OpenAI son aleatorias en base62 o base64url, así que tienen las tres clases; un identificador, una ruta o un nombre de paquete casi nunca lleva mayúsculas. Es una regla de una línea en `accepts`, sin tocar los otros patrones ([diseño, D1](design.md#d1-el-criterio-para-sk-)).
- **La marca es `<private>…</private>`**, sin distinguir mayúsculas, y se reemplaza entera (con las marcas) por `[REDACTED:private]`. Una marca sin cerrar enmascara hasta el final del texto, como ya fijó el mapa. Una marca que abre justo después de una comilla invertida (`` `<private>` ``) es texto, no marca: así se puede escribir sobre la marca en un save o en un `.md` sin perder lo que sigue ([D2](design.md#d2-la-marca)).
- **La marca vale también en el índice de docs**, porque la función es una sola (`crates/clawg-core/src/memory/project_docs/mod.rs:382`).
- **Patrones por contexto, versión acotada** (tarjeta, opción recomendada). Sólo el valor después de una clave que termina en `password` o `passwd` (`password=`, `DB_PASSWORD:`, `"password": "…"`) y el de `Authorization: Bearer|Basic|Token`. El valor se enmascara si es un literal de al menos 6 caracteres con un dígito o un símbolo, y no es una variable, una plantilla, una llamada ni un camino con puntos. `secret`, `token`, `api_key` y `pwd` quedan fuera: chocan con tipos (`token: string`), con código y con `PWD=/home/…` ([D3](design.md#d3-los-patrones-por-contexto)).
- **Formatos nuevos: cuatro con formato documentado y prefijo fijo.** Token de GitLab (`glpat-`), de npm (`npm_`), secreto de cliente OAuth de Google (`GOCSPX-`) y clave de SendGrid (`SG.….…`). Hugging Face (`hf_`) queda fuera: sus 34 letras no se distinguen de un nombre de clase como `AutoModelForSequenceClassification` ([D4](design.md#d4-formatos-nuevos)).
- **La clave privada sin línea final sigue enmascarando hasta el final.** Es la opción segura: limitarla a un largo máximo dejaría pasar la cola de una clave pegada sin su `END`. El costo, que una explicación del formato PEM que nombra la cabecera pierde lo que sigue, se acepta.
- **No se fuerza un reindexado de docs.** El índice se deriva del disco y no destruye nada del usuario: un pasaje indexado con las reglas viejas queda así hasta que su archivo cambie. Un archivo con un secreto ya lo tiene en claro en el repo del usuario.
- **La marca no se anuncia todavía en las herramientas MCP.** Decir cuándo usarla es de memory-protocol (22); este cambio no toca `agent_contract.json`.

## Alcance

1. Tabla de casos en los tests de `redact.rs`: claves sintéticas de cada formato, que se enmascaran, e identificadores, rutas, hashes y marcadores de posición, que no.
2. El criterio de mayúscula, minúscula y dígito para `openai_key`.
3. La marca `<private>` y su tipo `private`.
4. Los tipos `password` y `auth_header` (patrones por contexto, versión acotada).
5. Los tipos `gitlab_token`, `npm_token`, `google_oauth_secret` y `sendgrid_key`.
6. `.claude/CLAUDE.md`: qué se enmascara.

## Fuera de alcance

- Reescribir filas ya guardadas. `clawg secrets` las va a listar con los tipos nuevos (clasifica con la misma función, `crates/clawg-core/src/memory/secrets.rs:46`) y el usuario decide un `purge`.
- Recuperar los textos que ya se perdieron por un falso positivo: no queda el original en ningún lado.
- Detección por entropía sin contexto, guardar el original cifrado, avisar en lugar de enmascarar (descartados en la [exploración](exploration.md#descartado)).
- Contarle la marca al agente (memory-protocol, 22).
- Mover el enmascarado al handler (ingest-outbox, 36): las reglas nuevas aplican igual, porque es la misma función.

## Datos guardados

No cambia el esquema ni ningún identificador guardado. Cambia lo que entra desde ahora: un texto que antes se guardaba con un identificador `sk-…` enmascarado ahora se guarda entero, y uno con `password=…` o `<private>` ahora se guarda enmascarado. Las filas existentes no se tocan.

Dos efectos sobre filas viejas, los dos a propósito:

- `clawg secrets` lista las filas guardadas antes que tengan un patrón nuevo. Es la forma de encontrarlas.
- `clawg undelete` (`restore_memory_core`, `crates/clawg-core/src/memory/admin.rs:452-455`) se niega a restaurar una fila borrada que ahora tiene un secreto, como ya hace con los tipos de hoy.

Un save repetido después del cambio puede dar un hash de dedup distinto del de la fila vieja (el texto enmascarado cambió), y entonces se guarda como fila nueva en lugar de `reinforced`. No se pierde nada.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- `the sk-learn-pipeline-for-text-classification-v2-2026-model` queda sin tocar;
- toda clave sintética con formato válido sigue enmascarada, una por tipo, viejos y nuevos;
- `<private>x</private>` queda `[REDACTED:private]`; una marca sin cerrar enmascara hasta el final; dos marcas, dos reemplazos; `` `<private>` `` no cambia;
- `password=hunter2` y `Authorization: Bearer <token opaco>` enmascaran sólo el valor; `password=<tu-clave>`, `password: string`, `password = os.getenv("X")` y `Authorization: Bearer $TOKEN` no cambian;
- `masking_twice_finds_nothing_new` sigue valiendo con todos los tipos;
- `clawg secrets` lista una fila guardada antes con un patrón nuevo.

`rag-eval` no aplica: no cambia el ranking. En la máquina de Marcos, `clawg secrets` sobre el perfil dev con el binario nuevo es la medición sobre datos reales: cada fila nueva que lista es un secreto que se había guardado en claro o un falso positivo nuevo que hay que mirar ([`verify.md`](verify.md)).
