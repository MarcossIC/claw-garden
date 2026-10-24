# topic-key-warning — Especificación

## R1. La candidata dice si una clave del save la alcanza

Cada candidata del detector (`Candidate`) MUST llevar `same_scope`: verdadero cuando la fila es del proyecto del save, o cuando ni la fila ni el save tienen proyecto, que es el alcance de una clave (`SCOPE_ROWS_SQL`). Una fila sin proyecto, o una `user_preference` de otro proyecto, que el detector también alcanza, MUST llevarlo falso. El campo no cambia qué pares se escriben ni su estado.

## R2. El save sin clave nombra la fila que más probablemente revisa

Un save confirmado sin `topic_key` MUST nombrar en `revision_candidate` la candidata más cercana con `same_scope`, y de dos igual de cercanas la primera que encontró el detector. Un save con clave, o uno sin candidatas de su alcance, MUST NOT nombrar ninguna, y la respuesta de `POST /v1/memories` MUST omitir el campo. Un save que queda en cola (`202`) no lo nombra. La decisión MUST ser una función pura de la clave y las candidatas (`unkeyed_revision`).

#### Escenario: la más cercana de su proyecto
- GIVEN tres filas del agente: una del proyecto `p-1` a distancia 0,3 del save, otra sin proyecto a 0,15 y otra de `p-1` a 0,2
- WHEN el agente guarda sin clave en `p-1`
- THEN las tres son candidatas, la sin proyecto con `same_scope` falso
- AND `revision_candidate` es la de `p-1` a 0,2, no la más cercana de todas

#### Escenario: con clave
- GIVEN una fila del agente en `p`
- WHEN el agente guarda bajo `build/tool` un texto cercano en `p`
- THEN la respuesta nombra la fila en `related` y no lleva `revision_candidate`

#### Escenario: un save sin proyecto
- GIVEN filas del agente sólo en proyectos
- WHEN guarda sin proyecto ni clave un texto cercano
- THEN ninguna candidata tiene `same_scope` y no hay `revision_candidate`

**Tests:** `an_unkeyed_save_revises_its_closest_candidate_of_its_own_scope` (`topic.rs`); `an_unkeyed_save_names_the_closest_candidate_its_key_could_revise` y `a_save_under_a_key_or_without_a_candidate_of_its_scope_names_none` (`worker.rs`); `a_confirmed_save_names_its_revision_candidate_only_when_it_has_one` (`daemon/api.rs`); `an_unkeyed_save_names_the_row_a_key_would_revise` (`tests/api.rs`).

## R3. `save_memory` avisa con la línea medida

Cuando la respuesta trae `revision_candidate`, `save_memory` MUST agregar, en una línea propia y después de la de `close to earlier memories`, `this save has no topic_key: if it revises #N, save it again under a topic_key naming their subject, so the next revision replaces it`, con `N` la fila nombrada. Sin el campo (un save con clave, sin candidata de su alcance, o un daemon anterior) MUST NOT agregarla. La línea MUST ser la que midió `revision-eval` (`WARNING_LINE`, `cost.rs`) con el id de la fila.

#### Escenario: la línea
- GIVEN un save confirmado como `#57` con `related` `#412 "Build tool"` y `revision_candidate` 412
- THEN el texto es `saved #57`, la línea `close to earlier memories: #412 "Build tool"…` y, en la última línea, el aviso con `#412`

**Tests:** `an_unkeyed_save_close_to_its_own_row_is_told_to_take_a_key` (`server.rs`); `save_memory_ships_the_warning_line_that_was_measured` (`crates/clawg-eval/src/bin/revision-eval/cost.rs`).

## R4. El par registra que el aviso disparó

Cuando el worker nombra `revision_candidate` para un save, el par de `memory_conflict` que une el save con esa fila MUST quedar marcado (`topic_key_warned`), en la misma transacción que termina el save (`outbox_finish` o `finish_save`). En un save de varias piezas, cada pieza que el detector emparejó con esa fila tiene su par, y todos quedan marcados (`mark_warned_in`, `topic.rs`). Ningún otro par del save MUST quedar marcado, y un save que falla MUST NOT marcar nada. La marca dice que el detector disparó, no que el agente vio la línea: un save que respondió `202` también la lleva. Una vez escrita, la marca MUST NOT cambiar: no la quitan un veredicto, ni una clave que la fila tome después, ni un borrado. Sólo un `purge` de una de las dos filas se lleva el par entero, como con cualquier otro par. V19 MUST agregar la columna sin marca en los pares que ya existen. El export MUST llevarla en la línea `conflict` (`topic_key_warned`), y la importación MUST conservarla; una línea sin el campo MUST valer sin marca.

#### Escenario: dispara
- GIVEN dos filas del agente en `p`, una a distancia 0,2 del save y otra a 0,3
- WHEN el agente guarda sin clave en `p`
- THEN el par con la de 0,2, que es `revision_candidate`, queda marcado, y el par con la de 0,3 no

#### Escenario: seguir el aviso no borra la marca
- GIVEN el save de arriba, con su par marcado
- WHEN el agente guarda el mismo texto bajo `build/tool`
- THEN la fila del save toma la clave y el par sigue marcado
- AND el export muestra la fila con `topic_key` `build/tool` y el par con `topic_key_warned`

#### Escenario: con clave
- GIVEN una fila del agente en `p`
- WHEN el agente guarda bajo `build/tool` un texto cercano en `p`
- THEN el par se escribe sin marca

#### Escenario: una línea anterior
- GIVEN un export cuya línea `conflict` no tiene `topic_key_warned`
- WHEN se importa
- THEN el par queda sin marca

**Tests:** `an_unkeyed_save_marks_the_pair_with_its_revision_candidate`, `a_save_under_a_key_marks_no_pair`, `following_the_warning_keeps_the_pair_marked` y `a_verdict_or_a_delete_leaves_the_mark` (`worker.rs`); `a_store_from_before_the_warning_mark_keeps_its_pairs_unmarked` (`db.rs`); `a_store_from_before_the_warning_mark_exports_its_pairs_unmarked` (`export.rs`); `the_export_carries_the_warning_mark_and_the_import_keeps_it` (`import.rs`); `a_conflict_line_without_the_mark_reads_unmarked` (`format.rs`).

## R5. `revision-eval warned` mide el aviso sobre los pares marcados

Es la herramienta de la verificación: aplica el [criterio](proposal.md#criterio-fijado-antes-de-medir) a un export real. Va al lado de `store`, que queda como lo midió 48. Las rutas sin prefijo están en `crates/clawg-eval/src/bin/revision-eval/`, y la lógica de los dos pasos es pura (`warned.rs`), probada sin modelo con exports armados a mano.

```
revision-eval warned --export FILE --since SNAPSHOT [--seed N] (--real | --fake)
revision-eval warned --labels FILE [--tag SLUG]
```

### Primer paso: `--export`

**Entrada.**
- El export MUST venir de `clawg export --include-deleted`, sin `--project` ni `--agent` (`Filter`). Si no, se rechaza con 2 sin crear nada. Con un borrado que falte se pierden los pares de esa fila, y con un filtro, parte del store.
- `--since` MUST ser el nombre del snapshot `pre-migration` que escribió la apertura que migró el store a V19 (`memory-<UTC>-pre-migration.db`; la hora la lee `parse_snapshot_time`, `crates/clawg-core/src/memory/backup.rs`). También vale su ruta, de la que se toma sólo el nombre. Cualquier otro nombre se rechaza con 2 antes de leer el export, así que un error de tipeo no borra nada.
- La ventana son los saves desde ese instante. Antes ningún clawg escribía la marca, así que un save anterior se leería como uno en que el aviso no disparó.

**Población.**
- **Marcados:** cada par con `topic_key_warned` cuyo save (el que tiene la fila `new_id`) cae en la ventana. Se compara el save entero de `old_id` (el anterior) con el save avisado (el posterior).
  - Los pares de las piezas de un mismo save cuentan como un solo par marcado, el de menor distancia, y los demás se cuentan en `marks_merged`.
  - Una marca que el export no puede ubicar se cuenta en `marked_skipped`, y su save tampoco entra en el resto, porque el aviso disparó igual. Eso pasa cuando falta una de sus filas, cuando las dos filas son del mismo save o cuando el save no tiene agente.
- **Vivo en un momento:** una fila está viva desde que se hizo y hasta que se borró o se reemplazó. Un reemplazo por revisión ocurre cuando se hizo la fila que la reemplaza. Uno por veredicto ocurre cuando se juzgó su par, o si no hay par juzgado, cuando se hizo la fila ganadora. Una fila reemplazada por otra que el export no tiene no cuenta como viva para ningún save posterior. Un save está vivo si alguna de sus filas lo está.
- **El resto:** los saves de la ventana que cumplen todo esto:
  - tienen agente y son de un tipo que afirma;
  - no tienen clave en el export ni un par marcado;
  - tienen un save anterior de su agente y proyecto que estaba vivo en su momento, o sea ni borrado ni reemplazado antes.

  Un save con clave y sin par marcado cuenta como guardado con clave. MUST elegir 100 al azar con la semilla, o todos si hay menos.
- **El par de uno del resto** es el save anterior vivo más cercano, como en 48. La corrida guarda en orden todos los saves del export por el worker, en un store temporal, y mide las distancias entre sus filas (`nearest_earlier`, `run.rs`). Un save del resto sin un par que se pueda medir queda afuera, y se cuenta.

**Lo que deja.**
- `labels.jsonl` mezcla los pares marcados y los del resto, numerados, sin nada que diga cuál disparó. Cada línea lleva `n`, `earlier`, `later` y tres campos vacíos: `a` y `b` para las dos pasadas ciegas, y `c` para la tercera, que sólo resuelve desacuerdos.
- `warned.json` guarda lo que el segundo paso necesita de cada par, sin texto:
  - si es marcado y su distancia;
  - el tamaño del store en el momento del save, que son los saves del export hechos antes y vivos en ese momento;
  - en los marcados, tres acciones del agente leídas del export:
    - `followed`: la fila del save avisado tiene clave;
    - `hid`: la fila nombrada quedó `superseded` por veredicto a favor de una fila del save avisado;
    - `reused`: un save posterior del mismo agente y proyecto lleva la clave de la fila avisada.

  También guarda los conteos de la ventana: sus saves, los marcados y el resto.
- **Privacidad:** el export, las etiquetas, `warned.json` y el store de la reproducción MUST quedar en un directorio `revision-eval-*` hijo directo del temporal del sistema, como en `store` (`work_dir`, `run.rs`). El store de la reproducción se borra al final del paso, y ante un error se borra el directorio entero.

### Segundo paso: `--labels`

**Etiquetas.**
- Cada par MUST tener `a` y `b`, con `revision` u `other`, y también `c` donde `a` y `b` difieren. Si falta alguno, el paso se rechaza y el directorio queda, porque el etiquetado está en él.
- La etiqueta final es la que comparten `a` y `b`, o `c` cuando difieren.
- El acuerdo es la kappa de Cohen entre `a` y `b`, con su intervalo del 95 % por el error asintótico `sqrt(p_o (1 − p_o) / n) / (1 − p_e)`.

**Medidas.**
- **Precisión:** las revisiones entre los marcados, con su intervalo de Wilson.
- **π:** como en 48 (`prevalence`, `store.rs`). Los marcados son un censo y el resto una muestra, o también un censo si entró entero.
- **`p_min = 2π / (1 + π)`**.
- La sensibilidad, la tasa de falsas alarmas y su razón, con el resto estimado a partir de la muestra.

**Decisión.** `decision` vale:
- `keeps` si la cota inferior de la precisión supera `p_min`;
- `adjust_or_remove` si la cota superior queda por debajo;
- en otro caso, `keeps_by_default` con 200 marcados o más (`N_MAX`), y `gather_more` con menos.

**Por distancia.** Los marcados se parten en tercios como fija la propuesta. Cada tercio lleva su rango, su precisión y si acierta, es decir si su cota inferior supera `p_min`. Con `adjust_or_remove`, `narrow_to` da la distancia más alta del tercio más cercano si ese tercio acierta. Si no acierta, la decisión dice `remove`.

**Condiciones que siguen de 48.** Se informan aparte de `decision`:
- la línea del aviso cuesta como mucho 40 tokens;
- la media es como mucho 15 tokens por save, que es la línea por la tasa de disparo sobre los saves de la ventana;
- la prevalencia es de al menos 10 %, con su cota inferior de al menos 5 %.

**Lo que hizo el agente.** Se informa por separado entre los aciertos y entre las falsas alarmas, cada cifra con Wilson:
- `followed` y `hid`, sobre los marcados de ese grupo;
- `reused`, sobre los que siguieron el aviso.

**Por tamaño del store.** Tercios del tamaño sobre los saves de la ventana, con la tasa de disparo y la precisión de cada uno.

**Resultado.** `--real` MUST escribir `docs/eval/results/{fecha}-real-revision-eval-warned[-{tag}].json` y MUST NOT pisar uno que ya exista. Lleva sólo conteos, tasas, intervalos y los cortes de distancia y de tamaño, nunca texto, ids de fila ni claves. `--fake` sólo imprime.

**Tests:** en `warned.rs`, los de la entrada y la ventana, `an_export_made_without_the_deleted_rows_or_limited_to_part_of_the_store_is_refused`, `only_the_name_of_a_pre_migration_snapshot_gives_a_window_start` y `the_window_is_the_saves_made_from_the_snapshot_on`; los de la población, `a_marked_pair_compares_the_warned_save_with_the_save_of_the_old_row`, `a_marked_pair_the_export_cannot_place_is_skipped_and_counted`, `the_marks_of_the_pieces_of_one_save_are_one_marked_pair_at_the_least_distance`, `a_marked_pair_without_a_distance_is_refused_without_naming_it`, `the_store_size_counts_the_saves_live_when_the_save_came`, `the_rest_is_the_unkeyed_claims_of_the_window_with_a_live_earlier_save_of_their_scope`, `the_sample_of_the_rest_is_a_hundred_saves_or_all_of_them`, `the_same_seed_chooses_the_same_pairs_in_the_same_order` y `every_marked_pair_is_picked_and_mixed_in_with_the_rest`; los de las acciones del agente, `a_warning_is_followed_when_the_warned_save_took_a_key`, `a_key_is_reused_by_a_later_save_of_the_same_agent_and_project_under_it`, `the_named_row_is_hidden_when_a_verdict_replaced_it_in_favour_of_the_warned_save` y `a_verdict_in_favour_of_any_piece_of_the_warned_save_hides_the_named_row`; los de lo que deja el primer paso, `the_closest_predecessor_is_the_save_whose_row_lies_nearest_to_a_row_of_the_save`, `the_labels_file_does_not_say_which_pair_fired`, `a_pair_record_keeps_the_distance_and_the_actions_of_the_marked_pairs_only`, `a_save_of_the_rest_the_replay_could_not_pair_is_left_out_and_counted` y `a_marked_pair_is_labelled_whatever_the_replay_found`; y los del segundo paso, `a_labels_file_is_refused_where_a_pass_is_empty_or_wrong_and_the_directory_stays`, `a_disagreement_needs_the_third_pass_and_it_decides`, `cohens_kappa_comes_with_its_asymptotic_interval_and_is_null_when_the_passes_cannot_differ`, `the_precision_is_the_revisions_among_the_marked_with_their_wilson_interval`, `the_minimum_precision_comes_from_the_prevalence_of_the_marked_pairs_and_the_rest`, `sensitivity_and_false_alarms_estimate_what_the_warning_missed_from_the_sample_of_the_rest`, `the_warning_stays_when_the_lower_bound_of_the_precision_clears_the_minimum`, `the_warning_is_adjusted_or_removed_when_the_upper_bound_falls_below_the_minimum`, `an_undecided_precision_waits_for_more_cases_until_n_max_and_then_keeps_the_warning`, `the_decision_serializes_as_its_four_names`, `tertiles_cut_at_the_rounded_up_thirds_and_a_tie_goes_with_the_band_before`, `each_distance_band_reports_its_range_its_precision_and_whether_it_clears_the_minimum`, `a_rule_that_only_works_up_close_is_narrowed_to_the_range_of_the_closest_third`, `a_rule_the_closest_third_does_not_save_is_removed`, `a_marked_pair_without_a_distance_in_the_replay_file_is_refused`, `the_store_size_bands_give_the_fire_rate_and_the_precision_of_each_third`, `the_token_and_prevalence_gates_are_reported_apart_from_the_decision` y `what_the_agent_did_is_counted_apart_for_the_hits_and_the_false_alarms`; en `run.rs`, `a_fake_warned_run_counts_and_labels_the_marked_pairs_and_the_rest_of_a_small_export`, `the_same_seed_labels_the_same_pairs_in_the_same_order`, `an_export_the_first_step_refuses_is_removed_with_its_directory`, `the_second_step_scores_the_labels_and_removes_the_directory`, `a_labels_file_the_owner_has_not_finished_keeps_the_directory_of_the_second_step` y `the_second_step_removes_the_directory_when_the_replay_file_or_the_result_fails`; y en `main.rs`, `the_arguments_of_warned_parse`, `a_result_of_the_real_model_goes_to_a_file_that_is_not_there_yet` y `the_results_file_of_warned_holds_no_stored_text`.
