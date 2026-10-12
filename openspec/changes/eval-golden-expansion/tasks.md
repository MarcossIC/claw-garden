# eval-golden-expansion — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código; compila y deja `cargo test --workspace --target-dir <dir>` en verde. R* = requisitos de `spec/eval/spec.md`, D* = `design.md`. Rutas bajo `crates/clawg-eval/src/bin/`. Nunca `git add -A`; `docs/eval/**` no se commitea.

## 1. Test autocontenido (R6, D6)

- [x] 1.1 RED: `report.rs:851` falla hoy porque lee `docs/eval/results/2026-08-15-fake.json`, que no existe.
- [x] 1.2 Reemplazar el archivo por un `serde_json::json!` inline (sin `agent_view`, caso sin `subset`); afirmar que `diff_values` no falla y el bloque cuenta 1 caso.
- Commit: `test(eval): make the results-diff test self-contained`

## 2. Etiqueta `subset` al parsear (R3, D4)

- [x] 2.1 RED en `rag-eval/golden.rs`: sin `subset` parsea `natural`; `"lexical"` parsea; `subsett` y un valor desconocido fallan.
- [x] 2.2 `enum CaseSubset { Natural, Lexical, MultiRef, HighImportance }` (`snake_case`, `#[default] Natural`) y `subset` con `#[serde(default)]` en `RetrievalCase`.
- Commit: `feat(eval): add an optional subset label to golden cases`

## 3. El resultado por caso lleva el subconjunto (R4, D4)

- [x] 3.1 RED en `report.rs`: un `CaseResult` sin `subset` deserializa como `natural`; uno con `lexical` hace ida y vuelta.
- [x] 3.2 `subset` en `CaseResult` (`#[serde(default)]`) y en `EmbeddedCase`; completarlo en los tres `CaseResult` de `rag-eval/main.rs` (`:420`, `:831`, `:1107`) y el fixture de `audit.rs:413`.
- Commit: `feat(eval): carry the subset into each case result`

## 4. Bloque `by_subset` (R4, D4)

- [x] 4.1 RED en `report.rs`: `aggregate_by_subset` arma un bucket por subconjunto presente, ordenado por nombre serializado, sin `known_red`; con sólo `natural` iguala al agregado global; el stdout lista los subconjuntos.
- [x] 4.2 `SubsetAggregate { subset, n_cases, aggregate }`, `aggregate_by_subset` (gemela de `aggregate_by_lang`, `:348`), `by_subset` tras `by_lang` y tabla de stdout tras la de idiomas (`:542-557`).
- [x] 4.3 Llenarlo en los tres `RetrievalResults` de `main.rs` (`:479`, `:873`, `:654` vacío).
- [x] 4.4 Test: un resultado previo sin `by_subset` sigue leyéndose y comparándose con `--diff`.
- Commit: `feat(eval): report retrieval metrics per case subset`

## 5. Subcomando `synth` con base intacta (R1, R5, D1, D5)

- [x] 5.1 RED en `golden-mine/main.rs` (`parse_args`): `synth --base --out --seed [--distractors]` se reconoce; el modo de minado actual no cambia; `--out` igual a `--base` es error.
- [x] 5.2 RED en `synth/mod.rs`: `generate` conserva las filas y casos naturales intactos; `SynthError` ante ref o consulta duplicada.
- [x] 5.3 Crear `synth/{mod,golden}.rs` (tipos `Golden/Row/Case/Pair` con `deny_unknown_fields`) y `SynthError` (thiserror); `generate` sólo copia la base.
- [x] 5.4 `main.rs`: lee la base, escribe `to_string_pretty` + `\n`, imprime sha256 de base y salida.
- Commit: `feat(golden-mine): add a synth subcommand that preserves the base set`

## 6. PRNG y distractores (R1, R2, D2, D3)

- [x] 6.1 RED en `synth/rng.rs`: SplitMix64 fija sus vectores para la semilla 0 (`0xE220A8397B1DCDAF`, …); `below(n)` es determinista.
- [x] 6.2 RED en `synth/mod.rs`: misma semilla da la misma salida, otra da otra; >= 500 distractores por defecto; el tamaño sólo reduce distractores; sin refs duplicadas; ref `s-dis-`.
- [x] 6.3 Crear `rng.rs`, `vocab.rs` (backticks y palabras >= 5 letras sin stopwords, por `(memory_type, lang)`) y `templates.rs`; mezclar dos filas del grupo; heredar tipo e idioma.
- Commit: `feat(golden-mine): generate deterministic distractors near real rows`

## 7. Subconjunto léxico (R2, D5)

- [x] 7.1 RED: >= 30 casos `lexical`; 8 hermanos por familia; el identificador consultado aparece sólo en la fila esperada; consultas únicas; `expected_ref` en el corpus.
- [x] 7.2 Familias léxicas con refs `s-lex-` y consulta con el identificador.
- Commit: `feat(golden-mine): generate identifier queries with near-identical siblings`

## 8. Multi-referencia e importancia alta (R2)

- [x] 8.1 RED: >= 10 casos con más de una `expected_refs`; >= 20 filas con importancia 8-10, >= 10 con >= 9; los mínimos se mantienen con tamaño pequeño.
- [x] 8.2 Generar `s-mr-` y `s-hi-`, subconjuntos `multi_ref` y `high_importance`.
- Commit: `feat(golden-mine): generate multi-reference and high-importance cases`

## 9. Integración (R1, R3, R4)

- [x] 9.1 Crear `clawg-eval/tests/synth_roundtrip.rs`: base inline, `golden-mine synth`, y `rag-eval --fake --golden <out>` sale 0 y trae `by_subset`; dos corridas con la semilla 7 son idénticas, 7 y 8 difieren. No lee `docs/eval` ni `~/.clawg*`.
- Commit: `test(eval): run the generated set through rag-eval end to end`

## 10. Verificación (**10.2 a 10.4 necesitan autorización**: `--real` lee el modelo de `~/.clawg-dev`)

- [x] 10.1 `cargo test --workspace --target-dir <dir>`: conteo contra 1.304, 0 fallos.
  - Medido sobre el historial reescrito: 1540 pasan, 0 fallos (11 suites).
- [x] 10.2 Ancla: `rag-eval --real` con `golden.json` solo reproduce R@5 0,7826 y MRR 0,5979.
  - Reproducida: R@1 0,4348, R@5 0,7826, R@10 0,8913, MRR 0,5979, nDCG@10 0,6693 (n=46, subconjunto `natural`). `--real` solo leyó el modelo en caché de `~/.clawg-dev/models`; el almacén fue una base temporal.
- [x] 10.3 Baseline nuevo: generar con una semilla fija y medir `--real` sobre `golden-expanded.json`; anotar los sha256 y las métricas por subconjunto.
  - Semilla 7, 500 distractores por defecto: 842 filas (48 de la base), 112 casos (46 de la base). Reproducible: una segunda generación da el mismo sha256.
  - sha256 base `d2d9d732...ed792a`; salida `c9bc6317433dd050c9249d318f6072d13485ecb8363c2f5a017ecbd19b5f7396`.
  - Global (n=112): R@1 0,5714, R@5 0,7143, R@10 0,7679, MRR 0,6739, nDCG@10 0,6957.
  - `natural` (46): R@1 0,1304, R@5 0,3043, R@10 0,4348, MRR 0,2169. `lexical` (30): R@1 0,9667, R@5 1,0000, R@10 1,0000, MRR 0,9833. `multi_ref` (12): R@1 0,4167, R@5 1,0000, MRR 1,0000. `high_importance` (24): todo 1,0000.
  - Por idioma: en (56) R@5 0,6429; es (56) R@5 0,7857.
- [x] 10.4 Criterio de salida: R@5 léxico >= 10 puntos por debajo del natural; si no, registrarlo como resultado válido. Revisar `similarity.mean_corpus_distractor_gap`.
  - No se cumple, y es un resultado válido: R@5 léxico 1,0000 frente a natural 0,3043 (el léxico queda 69,6 puntos POR ENCIMA, no por debajo). Los distractores hunden el subconjunto natural (0,7826 a 0,3043) y los identificadores del subconjunto léxico siguen recuperándose por el canal léxico.
  - `similarity.mean_corpus_distractor_gap` = 0,0503; `mean_expected_distractor_gap` = 0,0042. Similitud (solo coseno) global: R@5 0,7232, MRR 0,6795.
- [ ] 10.5 ~~Escribir `verify.md` versionado; `state.yaml` a `verify`.~~ Reemplazada por 18.6: la medición de 10.3 y 10.4 mostró que el set no discrimina (`design.md`, revisión 2026-10-12).

---

## Revisión 2026-10-12 (D7 a D10, R2, R3, R7, R8)

Los subconjuntos `lexical`, `multi_ref` y `high_importance` saturan, y el canal léxico no cambia ningún ranking (`design.md`, revisión). Las secciones 11 a 18 rehacen la forma de lo generado. Valen las mismas reglas: RED primero, un commit por sección, y ni el código ni los mensajes de commit nombran planes ni tareas. Los tests de determinismo de la sección 6 se mantienen; cualquier cambio de salida cambia el sha256, y eso es lo esperado.

## 11. Subconjunto `low_importance` (R3, D9)

- [x] 11.1 RED en `rag-eval/golden.rs`: `"subset": "low_importance"` parsea como `CaseSubset::LowImportance`; los valores existentes no cambian.
- [x] 11.2 RED en `golden-mine/synth/golden.rs`: `Subset::LowImportance` hace ida y vuelta como `low_importance`.
- [x] 11.3 Agregar la variante en los dos enums. `aggregate_by_subset` la ordena por nombre serializado sin cambios.
- Commit: `feat(eval): add a low-importance subset label`

## 12. Identificadores casi idénticos (D7)

- [x] 12.1 RED en `synth/mod.rs`: `near_identifiers` (pura) devuelve `n` identificadores distintos entre sí y del original, con el mismo prefijo, cuyos 4 dígitos son una permutación del original o un ±1 en el último dígito. Es determinista para una semilla.
- [x] 12.2 Implementar `near_identifiers` con `SplitMix64`. Si una permutación ya está tomada en el corpus, se descarta y se prueba la siguiente.
- Commit: `feat(golden-mine): derive near-identical identifiers from a seed identifier`

## 13. Familias léxicas con mención al pasar (R2, D7)

- [x] 13.1 RED: en cada familia, la esperada usa un marco de `mention_frames`; los 7 hermanos usan el marco de `lexical_frames` que la consulta parafrasea, con identificadores de `near_identifiers`. El identificador consultado aparece en una sola fila del corpus de salida (distractores incluidos). Siguen 30 casos y 8 filas por familia.
- [x] 13.2 `templates::mention_frames(kind, spanish)` (>= 2 por idioma y tipo); `lexical_families` arma la esperada con un marco de mención y los hermanos con `near_identifiers`.
- Commit: `feat(golden-mine): mention the queried identifier in passing beside near-identical siblings`

## 14. Distractores del largo de su grupo (R2, D8)

- [x] 14.1 RED en `synth/vocab.rs`: `Group` expone la mediana de largo de sus filas base.
- [x] 14.2 RED en `synth/mod.rs`: cada distractor alcanza la mediana de su grupo o encadena 3 marcos; ningún distractor comparte 6 palabras seguidas con una fila base; hay >= 4 marcos por `(memory_type, lang)`.
- [x] 14.3 Mediana en `Vocab::from_rows`; `distractors` encadena marcos del mismo par `(R, R')` hasta la mediana (tope 3); 4 marcos por tipo e idioma en `templates::distractor_frames`.
- Commit: `feat(golden-mine): match distractor length to the rows they imitate`

## 15. multi_ref sobre una entidad concurrida (R2, D9)

- [x] 15.1 RED: cada caso `multi_ref` tiene 8 filas que nombran su clave, todas en `corpus_refs`; `expected_refs` son los 2 o 3 aspectos que pide la consulta; las claves no se repiten ni aparecen en la base; >= 10 casos.
- [x] 15.2 `templates::aspect_catalog(spanish)` (8 pares fila/pregunta por idioma) y `templates::multi_aspect_queries(spanish)`; `multi_ref_topics` los usa.
- Commit: `feat(golden-mine): ask about a few aspects of an entity that has many`

## 16. Casos de importancia disputados (R2, R3, D9)

- [x] 16.1 RED: 12 casos `high_importance` (esperada en [8, 10], 5 competidoras de la misma clave en [3, 5]) y 12 `low_importance` (esperada en [3, 5], competidoras en [8, 10]); la consulta pide el aspecto de la esperada; en total >= 20 filas con importancia 8-10 y >= 10 con >= 9; los mínimos se mantienen con tamaño pequeño.
- [x] 16.2 Reemplazar `high_importance_rules` por la generación de las dos direcciones sobre `aspect_catalog`. Quitar `rule_frames` si queda sin uso.
- Commit: `feat(golden-mine): contest each importance case with rows about the same entity`

## 17. Intrusos por subconjunto en el audit (R7, D10)

- [x] 17.1 RED en `rag-eval/audit.rs`: `top_intruders(&[CaseResult])` (pura) da, por subconjunto, la fila no esperada más frecuente en el top-5 y su fracción de casos; la esperada de un caso no cuenta; un subconjunto sin intrusos no genera entrada; el orden es por nombre serializado. Casos `known_red` excluidos.
- [x] 17.2 Campo `top_intruders` en `AuditReport`, línea en el stdout del audit. No toca `Verdict`.
- Commit: `feat(eval): report the most frequent intruder per subset in the audit`

## 18. Integración y re-medición (**18.3 a 18.5 necesitan autorización**: `--real` lee el modelo de `~/.clawg-dev`)

- [x] 18.1 `synth_roundtrip.rs`: la salida generada trae los cinco subconjuntos en `by_subset`, y `rag-eval --fake --audit` incluye `top_intruders`. Commit: `test(eval): cover every generated subset end to end`.
- [x] 18.2 `cargo test --workspace --target-dir <dir>`: 0 fallos; anotar el conteo nuevo contra 1540.
  - 1566 pasan, 0 fallos (1540 + 26 tests nuevos). Sin warnings nuevos: los mismos 4 de `clawg-core` y los 2 de `rag-eval` (`production_lexical_budget`, `ceiling`).
- [x] 18.3 Re-ancla: `rag-eval --real` con `golden.json` solo reproduce R@5 0,7826 y MRR 0,5979 (C1).
  - C1 se cumple: R@1 0,4348, R@5 0,7826, R@10 0,8913, MRR 0,5979, nDCG@10 0,6693 (n=46, `natural`). Sólo se leyó el modelo de `~/.clawg-dev/models`; el almacén fue una base temporal.
- [x] 18.4 Baseline nuevo: `golden-mine synth --seed 7` y `rag-eval --real --audit --golden <salida> --sweep-lexical 20 --sweep-lexical 0`, más una corrida con `--sweep-imp 1.0:1.0`. Anotar los sha256 de base y salida, las métricas por subconjunto, `top_intruders` y cuántos casos cambian de ranking entre el canal léxico encendido y apagado.
  - Semilla 7, 500 distractores: 1028 filas (48 de la base), 112 casos (46 de la base). Reproducible: una segunda generación da el mismo sha256.
  - sha256 base `d2d9d7320bce841126eac163790f1052d268521912cfcc6aac4f9af466ed792a`; salida `6312586c2206cc3156ba43f40f2025d176166f29f631118759abc9c3ffdd228c`.
  - Global (n=112): R@1 0,2247, R@5 0,5387, R@10 0,7679, MRR 0,3925, nDCG@10 0,4760. Por idioma: en (56) R@5 0,4911; es (56) R@5 0,5863.
  - Por subconjunto (R@1 / R@5 / R@10 / MRR / nDCG@10):
    - `natural` (46): 0,1304 / 0,3696 / 0,4348 / 0,2194 / 0,2713.
    - `lexical` (30): 0,2333 / 0,4333 / 1,0000 / 0,3825 / 0,5214.
    - `multi_ref` (12): 0,2639 / 0,7778 / 1,0000 / 0,7465 / 0,7686.
    - `high_importance` (12): 0,5833 / 1,0000 / 1,0000 / 0,7292 / 0,7968.
    - `low_importance` (12): 0,1667 / 0,7500 / 1,0000 / 0,3898 / 0,5342.
  - Audit: veredicto USABLE (saturación R@10 0,7679; acuerdo modal 0,0223; solapamiento léxico 0,1143; dominancia de importancia +0,0256). `similarity.mean_corpus_distractor_gap` 0,0572; `mean_expected_distractor_gap` -0,0082.
  - `top_intruders`: `natural` `s-lex-19-0` en 6 casos (13,0 %); `lexical` `s-lex-11-4` 2 casos (6,7 %); `multi_ref` `s-mr-03-1` 2 (16,7 %); `high_importance` `s-hi-04-5` 2 (16,7 %); `low_importance` `s-lo-05-2` 2 (16,7 %).
  - Canal léxico encendido (`--sweep-lexical 20`) frente a apagado (`0`): 0 de 112 rankings distintos (C6, diagnóstico).
  - Importancia aplanada (`--sweep-imp 1.0:1.0`), MRR: `high_importance` 0,5236 (R@5 1,0000), `low_importance` 0,5965 (R@5 0,9167), `natural` R@5 0,3478. Con la importancia por defecto la diferencia high − low es 0,3394; aplanada, −0,0729.
- [x] 18.5 Criterio de salida (R8): evaluar C2 a C5 contra 18.4 y registrar cada uno como cumplido o no, sin mover umbrales. El R@5 léxico de 18.4 es el `B` de los umbrales congelados de lexical-fusion.
  - C1 cumplido: R@5 0,7826 y MRR 0,5979.
  - C2 cumplido: `lexical` R@5 0,4333 (<= 0,70). `B` = 0,4333.
  - C3 cumplido: `multi_ref` R@5 0,7778 y nDCG@10 0,7686 (ambos <= 0,90).
  - C4 cumplido: MRR `high_importance` 0,7292 y `low_importance` 0,3898 (<= 0,90); diferencia 0,3394 con la importancia por defecto (>= 0,15) y −0,0729 aplanada (<= 0,05). Lectura: aplanada, el valor absoluto (0,0729) supera 0,05 y el signo se invierte; el umbral tal como está escrito se cumple, pero el par no queda tan neutro como en el prototipo (0,02).
  - C5 NO cumplido: en `natural`, `s-lex-19-0` aparece en el top-5 de 6 de 46 casos (13,0 % > 10 %). Se registra tal cual; no se itera el generador ni se mueve el umbral.
- [x] 18.6 Verificación final: escribir `verify.md` versionado (con 10.2 a 10.4 como medición previa y 18.3 a 18.5 como baseline vigente); `state.yaml` a `verify`.
  - `verify.md`: pasa con advertencias. 1566 pasan, 0 fallos; ancla, sha256 de la semilla 7 y métricas de 18.4 re-medidas sin diferencias. C5 fallado y aceptado; C4 cumplido sólo en la letra.
