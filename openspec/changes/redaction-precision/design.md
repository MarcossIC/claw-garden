# redaction-precision — Diseño

Las decisiones de la [propuesta](proposal.md), con las alternativas que se descartaron. Todo vive en `crates/clawg-core/src/memory/redact.rs`, que sigue siendo pura: ningún llamador cambia (`worker.rs:430`, `project_docs/mod.rs:382`, `secrets.rs:46`, `admin.rs:452`).

## D1. El criterio para `sk-`

**Elegido:** `accepts(OpenaiKey, s)` exige, además de no empezar con `ant-`, al menos una mayúscula, una minúscula y un dígito ASCII en la parte capturada.

Las claves de OpenAI (`sk-` con 48 caracteres de base62, y `sk-proj-`, `sk-svcacct-`, `sk-admin-` con base64url) son aleatorias: la probabilidad de que 48 caracteres de base62 no tengan ninguna mayúscula es (36/62)^48 ≈ 5·10⁻¹², y es menor para las más largas. Un identificador con guiones, una ruta o un nombre de paquete casi nunca lleva mayúsculas.

| Alternativa | Por qué no |
|---|---|
| Rechazar «palabras en minúscula separadas por guiones» (segmentos cortos) | Más reglas para el mismo efecto: sin mayúsculas ya se rechaza. Y una clave `sk-proj-` con muchos `-` o `_` podría parecer segmentada |
| Proporción de guiones | Un umbral sin datos para elegirlo; el base64url de una clave real también tiene guiones |
| Exigir un largo mínimo mayor que 20 | Un identificador largo lo pasa igual, y deja de enmascarar claves recortadas |

**Lo que sigue pasando:** un identificador con mayúsculas, minúsculas y dígitos después de `sk-` (`sk-Learn-Pipeline-V2-2026-model`) se sigue enmascarando. Es raro y queda fijado como caso conocido en la tabla.

## D2. La marca

**Elegido:** un escáner propio, `private_spans(text) -> Vec<Range<usize>>`, puro, cuyos rangos entran a `find_secrets` con el tipo `Private` y la misma regla de solapamiento que el resto.

1. Busca la próxima apertura `<private>` sin distinguir mayúsculas (`(?i)<private>`).
2. Si el carácter anterior es una comilla invertida, es texto: sigue buscando después de ella.
3. Si no, busca el primer cierre `</private>` después de la apertura. El rango va del `<` de la apertura al `>` del cierre, o hasta el final del texto si no hay cierre.
4. Sigue buscando después del rango.

Se reemplaza el rango entero, marcas incluidas, por `[REDACTED:private]`. Como el rango empieza antes que cualquier secreto de adentro, gana el solapamiento y la salida tiene un solo placeholder. Es idempotente: el placeholder no tiene marcas.

| Alternativa | Por qué no |
|---|---|
| Un patrón más en `PATTERNS`, `(?i)<private>(?s:.*?</private>\|.*)` | El crate `regex` no tiene lookbehind para la comilla invertida, y una apertura literal sin cerrar se comería el resto del texto antes de que `accepts` pudiera rechazarla: las marcas reales que siguen se perderían dentro de ese match |
| Conservar las marcas (`<private>[REDACTED]</private>`) | Más texto sin información, y dos formas de escribir lo mismo |
| Exentar también la marca entre comillas dobles o en bloques de código | Lo que se escribe sobre la marca en Markdown va entre comillas invertidas. Un bloque de código con la marca sin cerrar enmascara hasta el final del texto o del pasaje, y se acepta: es la regla segura que fijó el mapa |
| Marca sin cerrar que se deja pasar | La descartó el mapa: quien abre la marca dijo que lo que sigue es secreto |

## D3. Los patrones por contexto

**Elegido:** dos patrones con grupo de captura, que enmascaran sólo el valor, como `url_password`:

```text
password     (?i)\b\w*pass(?:word|wd)["']?\s*[:=]{1,2}\s*["']?([^\s"'`,;)}\]]+)
auth_header  (?i)\bauthorization["']?\s*[:=]\s*["']?(?:bearer|basic|token)\s+([^\s"'`,;)}\]]+)
```

`\w*` delante de la clave cubre `DB_PASSWORD`, `PGPASSWORD` y `new_password`; la clave tiene que terminar en `password` o `passwd`, así que `password_hash` o `passwordFile` no cuentan. `[:=]{1,2}` cubre `=`, `:`, `:=` y `==`.

`accepts` aplica a los dos la misma regla de valor, `is_literal_secret(v)`:

| Regla | Lo que deja afuera |
|---|---|
| al menos 6 caracteres | `?`, `None`, `&str` |
| no empieza con `$ % *` | `${DB_PASSWORD}`, `$TOKEN`, `%PASS%`, `******` |
| no tiene `( ) [ ] { } < >` | `os.getenv(`, `Option<String>`, `Optional[str`, `<tu-clave>`, `[REDACTED:…]` |
| no es un camino con puntos (`a.b`, `a.b.c`, de identificadores) | `process.env.SECRET`, `cfg.Password`, `settings.PASSWORD` |
| tiene un dígito o un símbolo que no es `-`, `_` ni `.` | `string`, `change-me`, `YOUR_TOKEN_HERE`, `correct-horse-battery` |

El último renglón acepta falsos negativos (una contraseña de sólo letras y guiones pasa) para no comerse tipos, nombres de variable y valores de ejemplo. El caso que queda del otro lado es una variable con dígitos (`password=pwd1`), que se enmascara: se pierde el nombre de una variable, no un texto del usuario.

| Alternativa | Por qué no |
|---|---|
| Sumar `secret`, `token`, `api_key`, `apikey` | Chocan con tipos (`token: string`, `secret: Secret<String>`), con nombres de campo y con texto sobre esos conceptos. Se pueden sumar después con su tabla de casos |
| Sumar `pwd` | `PWD=/home/user` en cada volcado de entorno |
| Enmascarar la clave y el valor | Se pierde el nombre de la variable, que es lo útil de la memoria |
| `Bearer` suelto, sin `Authorization` | Aparece en prosa y en documentación de APIs |

## D4. Formatos nuevos

| Tipo | Patrón | Fuente del formato |
|---|---|---|
| `gitlab_token` | `\bglpat-[A-Za-z0-9_-]{20,}` | token de acceso personal de GitLab |
| `npm_token` | `\bnpm_[A-Za-z0-9]{36}\b` | token de acceso de npm |
| `google_oauth_secret` | `\bGOCSPX-[A-Za-z0-9_-]{28}` | secreto de cliente OAuth de Google |
| `sendgrid_key` | `\bSG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}` | clave de API de SendGrid |

Los cuatro tienen un prefijo que no aparece en identificadores comunes. **Hugging Face (`hf_` y 34 letras) no entra:** no lleva dígitos, y un nombre de clase como `AutoModelForSequenceClassification` tiene justo 34 letras: con el prefijo delante, es indistinguible de un token. El escáner de secretos de GitHub lo confirma: rechazó un push de este mismo documento con esa cadena escrita junto al prefijo, tomándola por un token de Hugging Face.

## Orden y solapamientos

`PATTERNS` queda: los formatos de hoy, los cuatro nuevos antes de `jwt`, y al final `password` y `auth_header`. La marca entra por `private_spans`. La regla no cambia: gana el que empieza antes y, si empiezan juntos, el más largo; en un empate exacto, el orden de la lista. Así `Authorization: Bearer ghp_…` queda como `github_token` y no como `auth_header`, y `password=sk-proj-…` como `openai_key`.

## Tests

Una tabla de casos dentro de `mod tests` de `redact.rs`, con claves armadas en tiempo de ejecución (`repeat`) para que ningún literal del repositorio parezca una credencial a un escáner. No se agrega ninguna dependencia ni visibilidad.
