# eval-golden-expansion — Diseño

Este documento registra sólo las decisiones con alternativas reales. Los conteos y las reglas de aceptación los fija la spec.

## Contexto verificado

- `rag-eval` siembra **todo** el corpus en un store y busca sobre él completo (`crates/clawg-eval/src/bin/rag-eval/main.rs:176`, `:396-398`). `corpus_refs` sólo alimenta el bloque de similitud (`:239-249`). Una fila agregada al corpus compite con todas las consultas.
- `golden-mine` no comparte módulos con `rag-eval`: son dos bins sin librería (`crates/clawg-eval/Cargo.toml:11-26`), y ya define sus propios tipos de salida (`golden-mine/draft.rs:27-63`).
- `--diff` une los casos por el texto de `query` (`rag-eval/diff.rs:125-130`). Dos casos con la misma consulta se pisan.

## D1. Un archivo combinado o dos

| Opción | A favor | En contra |
|---|---|---|
| **a. El generador lee `golden.json` y escribe `golden-expanded.json` (natural + sintético)** | `rag-eval` no cambia de forma de carga: un `--golden`, un sha256 (`report.rs:148-153`) que cubre todo lo medido. `golden.json` queda intacto, así que la regresión (R@5 0,7826, MRR 0,5979) se mide sobre los mismos bytes | El archivo combinado depende de la base. Dos máquinas con distinto `golden.json` producen hashes distintos aunque usen la misma semilla |
| b. `synthetic.json` aparte, combinado en la carga con un flag nuevo | La parte sintética es portable entre máquinas | Dos hashes por corrida, un flag y una fusión nueva en `golden::load` (`golden.rs:194-206`). El hash registrado dejaría de describir lo medido |
| c. Sintético solo, sin filas naturales | Lo más simple | Los distractores no compiten con las filas reales, que es para lo que existen |

**Decisión: a.** El generador imprime el sha256 de la base y el de la salida. Rechaza `--out` igual a `--base`, para que la regresión nunca se mida sobre un archivo sobrescrito. La portabilidad entre máquinas de b no se gana de verdad: la parte natural ya es local por decisión del usuario.

## D2. Cómo se acercan los distractores sin LLM

| Opción | A favor | En contra |
|---|---|---|
| **a. Vocabulario sacado de la base en tiempo de generación + plantillas por `(memory_type, lang)`** | Comparten identificadores y términos con las filas reales, así compiten en el canal denso y en el léxico. El código no guarda contenido privado | La salida depende de la base (ya aceptado en D1) |
| b. Vocabulario fijo en el código | Independiente de la base | Habría que escribir en el repo los temas del corpus privado. Si no coinciden, los distractores se agrupan entre sí (riesgo de la propuesta) |
| c. Copias de filas reales con palabras cambiadas | Máxima cercanía | Una copia puede responder la consulta de verdad: el caso falla por una etiqueta incorrecta y no por el ranking |

**Decisión: a.** La forma de los distractores (largo y guardas) la reemplaza D8; la mezcla de dos filas del grupo sigue vigente.

- De cada fila natural se extraen los spans entre backticks (identificadores) y las palabras de 5 letras o más que no son stopwords (lista corta es/en en el código). Se agrupan por `(memory_type, lang)`.
- Cada distractor toma una fila R y otra R' del mismo grupo, y llena un marco de su tipo e idioma con un identificador y términos de cada una. Hereda `memory_type` y `lang` de R, y la importancia sale del rango del grupo, para que la importancia no los separe.
- Mezclar dos filas lo deja cerca de ambas sin responder ninguna. Si igual se agrupan entre sí, lo detecta el criterio de salida y lo muestra `similarity.mean_corpus_distractor_gap` en `--real`.

## D3. PRNG

`rand` 0.9 no es dependencia de `clawg-eval` (sólo de `clawg-cli`, `crates/clawg-cli/Cargo.toml:32`). Su documentación avisa que `StdRng` no es reproducible entre versiones, y los algoritmos de muestreo cambiaron valores entre 0.8 y 0.9. La reproducibilidad byte a byte necesitaría además `rand_chacha` como dependencia directa.

| Opción | A favor | En contra |
|---|---|---|
| **a. SplitMix64 escrito a mano (unas 15 líneas) en `synth/rng.rs`** | Estable por construcción y sin dependencias. Un test fija los primeros valores para la semilla 0 (`0xE220A8397B1DCDAF`, …) | Calidad estadística modesta, suficiente para elegir plantillas |
| b. `rand` + `rand_chacha` | API conocida | Dos dependencias nuevas. Una actualización de `rand` puede mover la salida sin que cambie el código |

**Decisión: a.** `below(n)` usa módulo: el sesgo es despreciable para n < 1000 y el resultado es determinista.

## D4. Etiqueta `subset` y métricas por subconjunto

| Opción | A favor | En contra |
|---|---|---|
| **a. `enum CaseSubset { Natural, Lexical, MultiRef, HighImportance }`, `snake_case`, `#[default] Natural`** | Un typo es error de parseo, como `CaseStatus` (`golden.rs:73-79`). `#[serde(default)]` mantiene parseando el `golden.json` actual, igual que `history` (`:63-70`) | Agregar un subconjunto requiere tocar código. Es lo esperable: cada uno trae su generador |
| b. `String` libre | Extensible sin código | Un `"lexcial"` armaría un bucket aparte en silencio |

**Decisión: a.**

- `CaseResult` suma `subset: CaseSubset` con `#[serde(default)]`, así los resultados viejos siguen leyéndose en `--diff`.
- `RetrievalResults` suma `by_subset: Vec<SubsetAggregate { subset, n_cases, aggregate }>` después de `by_lang` (`report.rs:102`), calculado por `aggregate_by_subset`. Es una función pura gemela de `aggregate_by_lang` (`:348-363`), ordenada por el nombre serializado para que el JSON sea comparable byte a byte.
- Sólo aparecen los subconjuntos presentes. Lo llenan los tres caminos que arman `RetrievalResults`: grilla (`main.rs:479`), `--follow-up` (`:873`) y `--agent-view` (`:654`, vacío). La tabla de stdout va después de la de idiomas (`report.rs:542-557`).
- Separar `--diff` por subconjunto queda para lexical-fusion. `CaseResult` ya trae la etiqueta, así que se puede hacer sin cambiar el formato.

## D5. Dónde vive el generador y cómo se separa del I/O

`golden-mine synth --base <path> --out <path> --seed <u64> [--distractors N]`. `parse_args` (`golden-mine/main.rs:63-94`) pasa a distinguir el subcomando. El modo actual de minado no cambia.

```
main (I/O) ── lee base ──→ synth::generate(&Golden, seed, &Sizes) -> Result<Golden, SynthError>
                                     │ puro: rng + vocab + plantillas
main (I/O) ←── to_string_pretty + '\n' ── escribe out, imprime sha256 de base y salida
```

- Los tipos `synth::golden::{Golden, Row, Case, Pair}` son propios de `golden-mine`, con `Serialize + Deserialize` y `deny_unknown_fields`. Si el esquema de `rag-eval` gana un campo, el generador falla al leer la base en vez de descartarlo. Los campos opcionales se omiten al serializar cuando tienen su valor por defecto.
- Esa duplicación tiene precedente (`draft.rs:27-28`). La cubre un test de integración que corre los dos binarios, descrito abajo.
- `SynthError` (thiserror): ref sintética que choca con una de la base, consulta duplicada (rompería el join de `--diff`), base sin filas para algún grupo.
- Prefijos de ref: `s-dis-`, `s-lex-`, `s-mr-`, `s-hi-`. Las consultas sintéticas incluyen el identificador o el nombre clave del grupo, así son únicas.
- Familias léxicas: 8 hermanos por familia. Con 4, los cuatro entran al top-5 aunque el orden sea al azar, y el R@5 no discrimina. **Reemplazado en parte por D7:** se mantienen los 8 por familia, pero cambia qué texto lleva cada miembro.

## D6. El test de `report.rs:851`

El archivo de disco se reemplaza por un `serde_json::json!` inline con la forma vieja: sin `agent_view`, sin `by_subset`, y un `per_case` cuyo caso no tiene `subset`. El lado nuevo lleva ese mismo caso. El test comprueba que `diff_values` no falla y que el caso se emparejó (el conteo del bloque es 1). Así también prueba el `#[serde(default)]` de D4. El path pasado a `diff_values` es sólo una etiqueta (`diff.rs:104-107`).

## Revisión 2026-10-12: el set generado no discrimina

### Qué se midió

`rag-eval --real` (multilingual-e5-small), semilla 7, 500 distractores, salida sha256 `c9bc6317…7396` (tarea 10.3). El ancla se reprodujo: `golden.json` solo da R@5 0,7826 y MRR 0,5979.

| Subconjunto | n | R@1 | R@5 | MRR |
|---|---|---|---|---|
| natural | 46 | 0,1304 | 0,3043 | 0,2169 |
| lexical | 30 | 0,9667 | 1,0000 | 0,9833 |
| multi_ref | 12 | 0,4167 | 1,0000 | 1,0000 |
| high_importance | 24 | 1,0000 | 1,0000 | 1,0000 |

Tres subconjuntos saturados no pueden mostrar una mejora ni una regresión. El criterio de salida 10.4 tampoco se cumplió: el léxico quedó casi 70 puntos **por encima** del natural.

Las hipótesis se probaron con variantes del JSON generado, armadas fuera del repo y medidas con el mismo binario (ninguna toca código). Cada fila de la tabla siguiente es una corrida `--real`.

| Variante | natural | lexical | multi_ref | high_imp. |
|---|---|---|---|---|
| set actual | 0,3043 | 1,0000 | 1,0000 | 1,0000 |
| base + sólo distractores | 0,3043 | — | — | — |
| base + sólo filas de casos sintéticos | 0,5217 | 1,0000 | 1,0000 | 1,0000 |
| distractores de largo doble | 0,4130 | — | — | — |
| léxico: identificadores casi idénticos (permutación de dígitos) | 0,3043 | 1,0000 | 1,0000 | 1,0000 |
| léxico: la esperada menciona el id al pasar | 0,3043 | 0,6667 | 1,0000 | 1,0000 |
| léxico: las dos cosas | 0,3043 | 0,3333 | 1,0000 | 1,0000 |
| lo anterior + entidad con muchas filas (multi_ref, importancia) | 0,3043 | 0,3333 | 0,7917 | 0,8750 |
| lo anterior + distractores de largo doble | 0,3478 | 0,3333 | 0,7917 | 0,8750 |

Los valores son R@5. La última fila es la forma que proponen D7 a D9.

### Hallazgo que condiciona todo: el canal léxico no cambia ningún ranking

En cada variante se corrió `--sweep-lexical 20 --sweep-lexical 0`. Las dos corridas dieron **0 rankings distintos** en los 112 casos de cada set, incluidos los léxicos difíciles, donde el identificador está sólo en la fila esperada.

La causa está en el código. Un candidato que sólo trae el canal léxico se puntúa con la misma fórmula y la misma distancia que uno denso (`crates/clawg-core/src/memory/search.rs:932-947`). Después compite por los `k_final` puestos ordenado por esa puntuación (`:958-964`). Una fila que quedó fuera de las 30 más cercanas en el KNN exacto tiene menos coseno que las 10 primeras. Sólo podría subir por el factor de importancia, que va de 0,85 a 1,0 (`memory/score.rs`, `ImportanceFactor`). Con el scorer actual, el canal léxico es casi inerte por construcción. Lo que un subconjunto léxico puede medir hoy es el **margen** que una fusión por rango podría recuperar, no una fortaleza.

### Hipótesis 1, léxico trivial: se refuta la causa propuesta y se confirma la trivialidad

- **No lo resuelve el canal FTS:** apagarlo no cambia nada. Lo resuelve el canal denso. Los 8 hermanos usan la misma frase y sólo cambia el identificador (`synth/mod.rs:151`; las frases están en `synth/templates.rs:83-134`). La consulta repite esa frase (`what does the error {id} mean` contra `The command failed with `{id}` because…`). Si todo lo demás es igual, los subtokens del identificador deciden solos.
  - En los 30 casos, los 120 puestos del top-5 que no son la esperada los ocupan hermanos de la misma familia. Ninguna fila del resto del corpus compite.
  - Ejemplo: `where is hydrateJournal6105 used`. Esperada `s-lex-00-1` en el puesto 1 y, detrás, los otros 7 hermanos.
- **Exigir que el identificador aparezca sólo en la esperada no es la causa.** Al contrario: es lo que hace que un oráculo FTS lo resuelva siempre, y por eso se mantiene (D7).
- **Los identificadores casi iguales solos no alcanzan:** con los 4 dígitos permutados entre hermanos, R@5 sigue en 1,0 (R@1 0,93). e5 distingue el orden de los dígitos.
- **¿Es una fortaleza del producto?** Muestra que el canal denso distingue identificadores cuando todo lo demás es igual. Es real, pero no es búsqueda híbrida, y un caso que siempre da 1,0 no detecta una regresión. Se deja de generar (D7, alternativa b).
- **El criterio "léxico 10 puntos por debajo del natural" no es una buena hipótesis.** Compara dos subconjuntos cuya dificultad la fija el generador: el natural cae o sube con los distractores, que son una perilla. No dice nada del scorer. Lo reemplaza un margen absoluto en D10.

### Hipótesis 2, multi_ref y high_importance saturan: confirmada

- `rag-eval` busca sobre el corpus completo (`rag-eval/main.rs:176`, `:400`). `corpus_refs` no recorta lo que compite. La competencia real la decide cuántas filas comparten la clave de la consulta.
- En `multi_ref`, el nombre clave aparece sólo en sus 2 o 3 filas (`synth/mod.rs:200-239`, frases en `templates.rs:138-160`). La consulta lo nombra, así que las 2 o 3 entran al top-5 por construcción. El R@1 de 0,4167 es el máximo posible: 1/3 y 1/2 promediados.
- En `high_importance`, cada regla tiene su propio nombre clave y es la única fila que lo menciona (`synth/mod.rs:243-277`). Por eso sale primera con R@1 1,0, sin que la importancia intervenga.
- **La importancia casi no entra en juego en `rag-eval`.** Las filas se siembran con `created_at` = ahora, así que el piso de tier (`exp(-λ·edad)` ≈ 1) nunca actúa. La importancia sólo mueve el puntaje entre 0,85 y 1,0, y sin una fila de la misma clave con la que competir no decide nada.

### Hipótesis 3, natural de 0,78 a 0,30: presión real, pero por ruido genérico y no por cercanía

- **Las filas de casos sintéticos no son la causa principal:** los distractores solos ya dejan el natural en 0,3043. Las filas de los otros subconjuntos solas lo dejan en 0,5217.
- **Los distractores no son casi-copias ni respuestas discutibles.** En los 22 casos que pasan de acierto a fallo, de los 121 distractores que quedan por encima de la esperada, sólo 5 comparten un identificador con ella. Ejemplos:

| Consulta natural | Esperada (rango ancla) | Distractor que la supera |
|---|---|---|
| ¿Qué herramienta de línea de comandos escribimos en Rust y cómo está organizada por capas? | `c01`: "MSC is a multi-purpose CLI tool in Rust… Layered architecture: commands → core → platform → ui." (1) | `s-dis-0067`: "Decisión: `efficiency` se encarga de resilience; `monitoring` conserva commands. Se descartó la alternativa de cambiar improvements." |
| ¿Dónde quedó la lógica que arma la lista de intervinientes para enviar…? | `c02`: "Extracted `buildParticipantsForSubmit`… into pure functions…" (2) | `s-dis-0147`: "Trabajamos en arreglar dentro de `interview`, lo comparamos con `problema` y acordamos mantener person y annex sin cambios." |
| ¿De dónde salen automáticamente los miembros del órgano de gobierno en el anexo? | `c06`: "Agregada la sección 'Consejo de administración' al anexo moral con auto-replicate…" (2) | `s-dis-0028`: "Elegimos canalizar anexo por `cleanupAnnexShareholderTree` en lugar de `FormGroup<T>`…" (mezcla de `c06` y `c04`) |
| ¿Qué se entregó en la etapa donde definimos los contratos de dominio…? | `c09`: "Completed phase 1.5 of onboarding refactor — 5 domain contracts…" (3) | `s-dis-0442`: "Decisión: `domain` se encarga de onboarding; `productivity` conserva management…" |
| What documentation got brought back in line with the code…? | `c15`: "Synced `tablero-indicadores-frontend.md`…" (1) | `s-dis-0208`: "Error while handling `early`: digits failed after value; retrying `patchValue` did not fix watchchanges." |

- Ninguno respondería la consulta: son ensaladas de palabras gramaticalmente bien formadas.
- **Ganan por tres rasgos de forma, no por tema:**
  - **Largo.** La mediana es de 120 caracteres frente a 212 en la base. Duplicar el largo sube el natural de 0,3043 a 0,4130.
  - **Idioma.** 159 de 183 quedan por encima de la esperada en el idioma de la consulta. De 10 casos interlingües que acertaban, se pierden 8; de los 26 del mismo idioma, se pierden 14.
  - **Uniformidad.** Hay dos frases por `(memory_type, lang)` (`templates.rs:8-67`).
- La fila sintética que más se repite aparece en el top-5 de 6 de los 46 casos naturales (13 %): `s-lex-23-7` y `s-hi-13`. Son las filas cortas de los otros subconjuntos.
- **Veredicto: la presión es injusta en la forma, no en el contenido.** Es tamaño de store (548 filas frente a 48) más debilidades reales de e5-small (texto corto y genérico, preferencia por el propio idioma). Inflada por un largo irreal, no se puede leer como "lo que pasaría con 500 memorias reales". No se intenta llevar el natural a ningún valor objetivo: eso sería ajustar la vara al resultado. Se corrige la forma (D8) y se vigila con una métrica (D10).

## D7. Familias léxicas: mención al pasar y hermanos con identificador casi idéntico (reemplaza la forma de D5)

| Opción | A favor | En contra |
|---|---|---|
| **a. La esperada menciona el identificador dentro de otra frase. Los 7 hermanos usan la frase que la consulta parafrasea, con identificadores casi iguales** | Es el caso realista: la memoria que tiene el código exacto habla de otra cosa (un deploy, un refactor), y las parecidas a la pregunta nombran otro código. Medido: R@5 0,3333, R@10 0,80. Un oráculo FTS lo resuelve igual, porque el identificador es único | Con el scorer actual mide margen, no fortaleza. Es lo que lexical-fusion necesita |
| b. Mantener la frase compartida y sólo acercar los identificadores | Cambio mínimo | Medido: R@5 1,0. El denso distingue la permutación |
| c. Familias de más de 30 hermanos, para sacar a la esperada del pool KNN | Ataca el pool directamente | El candidato léxico se re-puntúa por coseno (`search.rs:932-964`): aunque el FTS lo traiga, no le gana a las 10 primeras. Sólo infla el corpus |
| d. Quitar la regla "el id está sólo en la esperada" | — | El caso deja de tener solución léxica y no mide el canal |

**Decisión: a.**

- `templates::mention_frames(kind, spanish)`: al menos 2 frases por `(IdentKind, idioma)`, con un marco distinto de `lexical_frames`. La consulta y la frase de los hermanos siguen saliendo de `lexical_frames`.
- Identificadores cercanos, con una función pura `near_identifiers(stem, digits, n, rng)`:
  - comparten el prefijo (`hydrateJournal`, `E_CACHE_`) con el de la esperada;
  - sus 4 dígitos son una permutación de los de ella, o difieren en ±1 en el último dígito;
  - nunca son iguales al de la esperada.
- Invariante que se mantiene y se amplía: el token FTS del identificador consultado aparece en **una sola fila de todo el corpus de salida**, distractores incluidos. Con `unicode61`, `E_CACHE_3698` es la frase `e cache 3698` y `hydrateJournal6105` es un solo token.
- Siguen 30 familias de 8 miembros.
- Tradeoff aceptado: el subconjunto queda por debajo del natural porque el scorer actual no puede promover candidatos léxicos. Es el baseline `B` contra el que lexical-fusion se mide con los umbrales congelados de la propuesta.

## D8. Distractores del largo de las filas que imitan, sin copiar ninguna (reemplaza la forma de D2)

| Opción | A favor | En contra |
|---|---|---|
| **a. Concatenar marcos del grupo hasta llegar a la mediana de largo de sus filas base (tope 3), con guarda anti-copia** | Saca el sesgo de largo, que medido valía 11 puntos. Sigue mezclando dos filas, así que no responde ninguna | No corrige la preferencia de idioma de e5. Es una debilidad real del modelo y no se esconde |
| b. Distractores = una fila base con sus identificadores cambiados por los de otra | Máxima cercanía temática | Conserva la prosa que responde la consulta original: es una respuesta discutible (D2 c) |
| c. Bajar la cantidad de distractores hasta que el natural vuelva cerca de 0,78 | Natural "sano" | Ajusta la vara al resultado. Además los candidatos sólo léxicos dejan de existir, que es para lo que están los distractores |
| d. Escribir en el código frases temáticas fijas | Prosa más natural | Pone temas del corpus privado en el repo (D2 b) |

**Decisión: a.**

- `Vocab` registra la mediana de largo (caracteres) de cada grupo.
- Un distractor encadena 1 a 3 marcos de su `(memory_type, lang)` con el mismo par de filas `(R, R')` hasta llegar a esa mediana.
- Sube la cantidad de marcos por `(memory_type, lang)` de 2 a 4, para que 500 filas no salgan de 24 frases.
- **Guarda anti-copia, como invariante testeado:** ningún distractor comparte una secuencia de 6 palabras seguidas con una fila base. Así ninguno contiene una oración de una fila real, que es lo que podría responder una consulta.
- Lo que no se corrige en el generador se vigila con la métrica de intrusos de D10.

## D9. multi_ref y high/low importance sobre una entidad con muchas filas

| Opción | A favor | En contra |
|---|---|---|
| **a. "Entidad concurrida": cada clave tiene un catálogo de aspectos (8 filas que la nombran) y la consulta pregunta por algunos** | Las otras filas de la misma clave compiten de verdad y son incorrectas sin discusión: hablan de otro aspecto. Medido: multi_ref R@5 0,7917 y MRR 0,8333; importancia R@5 0,8750 y MRR 0,5782 | Más filas por caso (unas 240 más) |
| b. Claves vecinas (`amber-heron` frente a `amber-falcon`) con las mismas frases | Pocas filas | Medido: sigue en 1,0. Es el mismo efecto de "todo igual menos la clave" que en D7 b |
| c. Más referencias esperadas por caso (5 o 6) | Satura menos el R@5 | Lo baja con un techo aritmético, no con competencia. No mide nada nuevo |

**Decisión: a, y el subconjunto de importancia se parte en dos direcciones.**

- `templates::aspect_catalog(spanish)`: 8 pares `(fila, pregunta)` por idioma, cada uno con `{key}` (retry, timeout, despliegue, reversión, acceso, dueño, almacenamiento, métricas). Más `multi_aspect_queries(spanish)` para 2 y 3 aspectos.
- `multi_ref`: 12 claves. Cada una suma las 8 filas a `corpus_refs`. `expected_refs` son los 2 o 3 aspectos que pregunta la consulta.
- Importancia: 24 claves de 6 filas cada una, en dos subconjuntos de 12 casos. La consulta pregunta un aspecto.
  - `high_importance`: la esperada tiene importancia 8 a 10 y las otras 5 filas, 3 a 5.
  - `low_importance`, nuevo: la esperada tiene 3 a 5 y las otras 5 filas, 8 a 10.
  - Ninguna de las dos direcciones satura (MRR 0,7569 y 0,3995). Con la importancia aplanada (`--sweep-imp 1.0:1.0`) las dos quedan en 0,55 y 0,54. La brecha de 0,36 es el efecto de la importancia, medido en las dos direcciones.
  - Importancia 8 a 10 queda en 72 filas, 56 de ellas con 9 o más. R2 se sigue cumpliendo.
- `CaseSubset` y `synth::golden::Subset` suman `LowImportance` (`low_importance`). D4 ya preveía que agregar un subconjunto toca código.
- Alternativa rechazada para no agregar la variante: partir `high_importance` según la importancia de la esperada en el reporte. Sería lógica implícita en `aggregate_by_subset`, y el JSON de casos no diría qué mide cada uno.

## D10. Criterio de salida falsable y métrica de intrusos (reemplaza el criterio 10.4)

**Métrica nueva en `--audit`: `top_intruders`.** Para cada subconjunto da la fila no esperada que aparece en el top-5 de más casos y la fracción de casos en que aparece.

- Es una función pura sobre `&[CaseResult]` en `audit.rs`, junto a `modal_rank_agreement`, que mide lo mismo a nivel de lista entera.
- Se reporta y no gatea el veredicto del audit: describe el corpus, no si el set es usable.
- Rechazado: calcularla en un script fuera del repo, porque no se reproduciría.

**Criterio de salida** (semilla 7, `--real`, sobre `golden-expanded.json`):

| # | Condición | Por qué ese umbral |
|---|---|---|
| C1 | `golden.json` solo da R@5 0,7826 y MRR 0,5979 | Ancla: el generador no toca la base |
| C2 | `lexical`: R@5 <= 0,70 | Margen para el umbral congelado de lexical-fusion: con `B` = 0,70 pide 0,85. Prototipo: 0,3333 |
| C3 | `multi_ref`: R@5 <= 0,90 y nDCG@10 <= 0,90 | Mismo techo que `MAX_SATURATION` de `audit.rs:35`. Prototipo: 0,7917 y 0,8185 |
| C4 | `high_importance` y `low_importance`: MRR <= 0,90 cada uno, y MRR(high) − MRR(low) >= 0,15 con la importancia por defecto, y |MRR(high) − MRR(low)| <= 0,05 con `--sweep-imp 1.0:1.0` | El par tiene que medir la importancia y nada más. Prototipo: 0,36 y 0,02 |
| C5 | `top_intruders` del subconjunto `natural`: ninguna fila sintética supera el 10 % de los casos | Al azar, una fila ocupa un puesto del top-5 en menos del 1 % de los casos. Hoy da 13 % |
| C6 | Diagnóstico, no gate: cuántos casos cambian de ranking entre `--sweep-lexical 20` y `--sweep-lexical 0` | Es el número que lexical-fusion tiene que mover. Hoy da 0 |

- El R@5 del natural no tiene umbral: depende del tamaño del corpus y del modelo. Se anota como baseline nuevo.
- Si C2 a C5 no se cumplen con los parámetros de D7 a D9, el resultado se registra igual y se decide si iterar el generador antes de proponer lexical-fusion. Los umbrales no se mueven después de ver la medición.

## Archivos

| Archivo | Acción |
|---|---|
| `crates/clawg-eval/src/bin/golden-mine/main.rs` | Subcomando `synth` y escritura |
| `crates/clawg-eval/src/bin/golden-mine/synth/{mod,rng,vocab,templates,golden}.rs` | Crear: generador puro |
| `crates/clawg-eval/src/bin/rag-eval/golden.rs` | `CaseSubset` y el campo `subset` |
| `crates/clawg-eval/src/bin/rag-eval/report.rs` | `subset`, `by_subset`, `aggregate_by_subset`, stdout, test inline |
| `crates/clawg-eval/src/bin/rag-eval/main.rs` | `subset` en `EmbeddedCase` y en los tres `CaseResult` que arma (`:420`, `:831`, `:1107`), y `by_subset` en los tres `RetrievalResults` |
| `crates/clawg-eval/src/bin/rag-eval/audit.rs` | Fixture de test (`:413`) con el campo nuevo |
| `crates/clawg-eval/tests/synth_roundtrip.rs` | Crear: test de integración |
| `crates/clawg-eval/src/bin/golden-mine/synth/{mod,templates,vocab}.rs` | Revisión: `mention_frames` y `near_identifiers` (D7); mediana de largo por grupo, marcos encadenados y 4 marcos por grupo (D8); `aspect_catalog`, `multi_aspect_queries` y los dos subconjuntos de importancia (D9) |
| `crates/clawg-eval/src/bin/golden-mine/synth/golden.rs` | Revisión: `Subset::LowImportance` |
| `crates/clawg-eval/src/bin/rag-eval/golden.rs` | Revisión: `CaseSubset::LowImportance` |
| `crates/clawg-eval/src/bin/rag-eval/audit.rs` | Revisión: `top_intruders` por subconjunto, en `AuditReport` y en stdout (D10) |
| `crates/clawg-eval/tests/synth_roundtrip.rs` | Revisión: los cinco subconjuntos llegan a `by_subset` |

## Tests

| Capa | Qué |
|---|---|
| Unit (`synth`) | Misma semilla da la misma salida, otra semilla da otra. SplitMix64 fija sus vectores. Se cumplen los conteos mínimos. No hay refs ni consultas duplicadas. Las filas naturales pasan intactas |
| Unit (`golden`, `report`) | Un caso sin `subset` parsea como `natural`. Un subset desconocido falla. `aggregate_by_subset` arma un bucket por subconjunto. D6 |
| Integración | Base inline → `golden-mine synth` → `rag-eval --fake --golden <out>` sale con 0 y trae `by_subset`. No se lee `docs/eval` ni `~/.clawg*` |
| Unit (`synth`, revisión) | D7: la esperada usa un marco de mención distinto del de sus hermanos; los identificadores de los hermanos comparten prefijo, son permutación o ±1 de los dígitos y nunca iguales; el identificador consultado está en una sola fila del corpus de salida. D8: cada distractor llega a la mediana de su grupo o usa 3 marcos; ninguno comparte 6 palabras seguidas con una fila base. D9: 8 aspectos por clave en `multi_ref`; en `high_importance` y `low_importance`, importancias de la esperada y de las competidoras según la dirección |
| Unit (`audit`, revisión) | `top_intruders`: la fila no esperada más frecuente en el top-5 por subconjunto, con su fracción; casos sin intrusos no generan entrada; las esperadas nunca cuentan |

## Rollout

No hay migración. `golden.json` no cambia. La medición `--real` lee el modelo del perfil dev y necesita autorización antes de correr.

## Preguntas abiertas

- [ ] Si tras la revisión C5 (intrusos en natural) no se cumple, ¿se itera el generador (más marcos, filas sintéticas más largas) o se acepta y se registra? Este diseño propone registrarlo y decidir con el número en la mano, sin mover el umbral.
- [ ] El `--diff` separado por subconjunto, que necesitan los umbrales de lexical-fusion: ¿va en este cambio o en lexical-fusion? Este diseño lo deja para lexical-fusion.
