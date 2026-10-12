# Especificación de eval (golden set y reporte)

## Purpose

Comportamiento del golden set de `rag-eval` y de su generador sintético en `golden-mine`. No existe spec previa de este dominio: es una spec completa. El parseo actual rechaza claves desconocidas (`crates/clawg-eval/src/bin/rag-eval/golden.rs:31,55`) y el reporte sólo parte por idioma (`report.rs:97-102`, `report.rs:348`).

## Requirements

### Requirement: Generador sintético determinista

`golden-mine` MUST ofrecer un modo generador que reciba una semilla y un tamaño y escriba bajo `docs/eval/`. Con la misma semilla y el mismo código MUST producir un archivo idéntico byte a byte; con otra semilla MUST producir otro. MUST NOT leer `~/.clawg*` ni la red.

#### Scenario: misma semilla
- GIVEN dos corridas con semilla 7 y el mismo tamaño
- WHEN se comparan los archivos generados
- THEN son idénticos byte a byte

#### Scenario: otra semilla
- GIVEN corridas con semillas 7 y 8
- WHEN se comparan los archivos
- THEN difieren

### Requirement: Composición mínima de lo generado

La salida SHALL incluir:

- **500 distractores por defecto.** Cada uno mezcla términos de dos filas reales del mismo `(memory_type, lang)`. Su largo MUST llegar a la mediana de largo de las filas base de su grupo, salvo que ya encadene 3 marcos. Ningún distractor MUST compartir una secuencia de 6 palabras seguidas con una fila base.
- **Al menos 30 casos `lexical`** (consulta por identificador, función o código de error), en familias de 8 filas:
  - la fila esperada menciona el identificador dentro de un marco distinto del que parafrasea la consulta;
  - los 7 hermanos usan ese marco, con identificadores del mismo prefijo cuyos dígitos son una permutación de los de la esperada o difieren en ±1 en el último dígito;
  - el identificador consultado MUST aparecer en una sola fila de todo el corpus de salida.
- **Al menos 10 casos `multi_ref`.** Cada clave tiene 8 filas de aspectos que la nombran; la consulta pide 2 o 3 aspectos y sólo esas filas son `expected_refs`.
- **Al menos 10 casos `high_importance` y 10 `low_importance`**, cada uno sobre una clave con 6 filas de aspectos y una consulta por un aspecto:
  - en `high_importance`, la esperada tiene importancia en [8, 10] y las otras 5 filas, en [3, 5];
  - en `low_importance`, la esperada tiene importancia en [3, 5] y las otras 5 filas, en [8, 10].
- En total, al menos 20 filas con `importance` en [8, 10], de ellas al menos 10 con `importance >= 9`.

Toda `expected_ref` MUST existir en el corpus del caso.

#### Scenario: conteos mínimos
- GIVEN una corrida con valores por defecto
- WHEN se cuenta la salida
- THEN hay >= 500 distractores, >= 30 casos léxicos, >= 10 multi-referencia, >= 10 `high_importance`, >= 10 `low_importance`, >= 20 filas con importancia 8-10 y >= 10 con >= 9

#### Scenario: hermano léxico
- GIVEN un caso léxico
- WHEN se comparan la fila esperada y sus hermanos
- THEN el identificador consultado aparece sólo en la esperada, y en ninguna otra fila del corpus de salida
- AND cada hermano tiene un identificador con el mismo prefijo, distinto del consultado, cuyos dígitos son una permutación o un ±1 en el último dígito
- AND el marco de la esperada no es el de los hermanos

#### Scenario: distractor sin copia
- GIVEN cualquier distractor
- WHEN se lo compara con las filas base
- THEN no comparte ninguna secuencia de 6 palabras seguidas con ninguna de ellas
- AND su largo es >= la mediana de su grupo, o encadena 3 marcos

#### Scenario: entidad concurrida
- GIVEN un caso `multi_ref`, `high_importance` o `low_importance`
- WHEN se miran las filas que nombran su clave
- THEN son más que las esperadas (8 en `multi_ref`, 6 en los de importancia), y las no esperadas tratan otro aspecto
- AND las importancias de la esperada y de las competidoras caen en los rangos de su subconjunto

#### Scenario: tamaño pequeño
- GIVEN un tamaño menor al por defecto
- WHEN se genera
- THEN los mínimos de casos y filas de importancia se mantienen y sólo se reduce el número de distractores

### Requirement: Etiqueta de subconjunto por caso

`RetrievalCase` MUST aceptar una clave opcional `subset` con los valores `natural`, `lexical`, `multi_ref`, `high_importance` y `low_importance`. Si falta, el caso MUST valer `natural`. La clave desconocida distinta de `subset` MUST seguir siendo un error de parseo.

#### Scenario: golden actual sin etiqueta
- GIVEN el `golden.json` actual, sin `subset`
- WHEN se parsea
- THEN todos los casos tienen subconjunto `natural`

#### Scenario: etiqueta explícita
- GIVEN un caso con `"subset": "lexical"`
- WHEN se parsea
- THEN el caso es `lexical`

#### Scenario: subconjunto de importancia baja
- GIVEN un caso con `"subset": "low_importance"`
- WHEN se parsea
- THEN el caso es `low_importance`

#### Scenario: clave mal escrita
- GIVEN un caso con la clave `subsett`
- WHEN se parsea
- THEN el parseo falla

### Requirement: Métricas por subconjunto en el reporte

El reporte de recuperación MUST incluir un bloque por subconjunto presente, con `n_cases` y el mismo `Aggregate` que el global, sobre los mismos casos activos, junto a `by_lang`. Los casos `known_red` MUST NOT entrar. Un resultado previo sin el bloque MUST seguir leyéndose y comparándose con `--diff`.

#### Scenario: dos subconjuntos
- GIVEN casos `natural` y `lexical`
- WHEN se arma el reporte
- THEN hay un bloque por cada uno, con R@5 y MRR calculados sólo sobre sus casos

#### Scenario: sólo natural
- GIVEN un golden sin casos etiquetados
- WHEN se arma el reporte
- THEN hay un único bloque `natural` con las mismas cifras que el agregado global

### Requirement: Ancla de regresión

Medir con `rag-eval --real` el `golden.json` actual sin filas sintéticas MUST reproducir R@5 0,7826 y MRR 0,5979. El generador MUST NOT modificar `golden.json`.

#### Scenario: sin sintéticos
- GIVEN el `golden.json` actual y ningún archivo generado
- WHEN se mide con `--real`
- THEN R@5 es 0,7826 y MRR es 0,5979

#### Scenario: generador no toca lo natural
- GIVEN un `golden.json` existente
- WHEN se ejecuta el generador
- THEN el contenido de `golden.json` queda intacto

### Requirement: Test de resultados autocontenido

El test de `report.rs:851` MUST pasar sin que exista nada bajo `docs/eval/`, usando un fixture inline que represente un resultado sin el bloque `agent_view`.

#### Scenario: sin docs/eval
- GIVEN un checkout sin `docs/eval/results/`
- WHEN corre el test
- THEN pasa y verifica que el diff entre el resultado sin bloque y otro con bloque funciona

### Requirement: Intrusos por subconjunto en el audit

Con `--audit`, el reporte MUST incluir, para cada subconjunto presente, la fila no esperada que aparece en el top-5 de más casos activos, con la fracción de casos del subconjunto en que aparece. Una fila esperada del caso MUST NOT contar como intrusa en ese caso. La métrica MUST NOT cambiar el veredicto del audit.

#### Scenario: un intruso repetido
- GIVEN 4 casos `natural` y una fila `s-dis-0001` en el top-5 de 2 de ellos sin ser esperada
- WHEN se audita
- THEN el bloque `natural` nombra `s-dis-0001` con fracción 0,5

#### Scenario: la esperada no es intrusa
- GIVEN un caso cuya fila esperada está en el puesto 1
- WHEN se audita
- THEN esa fila no cuenta para la fracción de intrusos de ese caso

### Requirement: Criterio de salida del set generado

Medido con `rag-eval --real` sobre la salida de la semilla 7 con valores por defecto, el set generado MUST cumplir:

| # | Condición |
|---|---|
| C1 | El ancla de regresión se reproduce |
| C2 | `lexical`: R@5 <= 0,70 |
| C3 | `multi_ref`: R@5 <= 0,90 y nDCG@10 <= 0,90 |
| C4 | `high_importance` y `low_importance`: MRR <= 0,90 cada uno. MRR(`high_importance`) − MRR(`low_importance`) >= 0,15 con la importancia por defecto, y su valor absoluto <= 0,05 con `--sweep-imp 1.0:1.0` |
| C5 | En `natural`, ningún intruso sintético supera el 10 % de los casos |

Además MUST anotarse, sin umbral, cuántos casos cambian de ranking entre `--sweep-lexical 20` y `--sweep-lexical 0`, y el R@5 de `natural`. Si una condición falla, el resultado MUST registrarse tal cual; los umbrales MUST NOT ajustarse después de medir.

Justificación (`design.md`, revisión 2026-10-12):

- Con el set de la tarea 10.3, `lexical`, `multi_ref` y `high_importance` dan R@5 1,0, y el canal léxico no cambia ningún ranking.
- C2 deja margen al umbral congelado de lexical-fusion: con `B` = 0,70 pide 0,85.
- C3 usa el techo de `MAX_SATURATION` (0,90).
- C4 exige que el par de importancia mida la importancia y nada más; por eso la brecha aplanada se mide en valor absoluto, en cualquiera de los dos sentidos. La redacción original (`<= 0,05` sin valor absoluto) dejaba pasar un sesgo a favor de `low_importance`; se corrigió después de medir y en el sentido más estricto (ver `verify.md`).
- C5 sale del 13 % medido, frente a menos del 1 % esperable al azar.

#### Scenario: subconjunto saturado
- GIVEN un set generado cuyo `lexical` da R@5 1,0
- WHEN se evalúa el criterio
- THEN C2 falla y el set no sirve de baseline para lexical-fusion

#### Scenario: par de importancia insensible
- GIVEN un set en que, con la importancia por defecto, MRR(`high_importance`) − MRR(`low_importance`) es menor que 0,15
- WHEN se evalúa el criterio
- THEN C4 falla: el par no mide la importancia
