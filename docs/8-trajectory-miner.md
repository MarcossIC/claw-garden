## 8. Trajectory Miner — Aprendizaje entre Sesiones

**Área:** Backend Rust — sub-agente de análisis de sesiones
**Fuente:** *AI: Research Memoria Persistente*

### Problema

Los agentes repiten los mismos errores sesión tras sesión porque los logs de sesiones pasadas se descartan. No hay mecanismo para extraer aprendizajes y codificarlos como reglas permanentes.

### Solución

Un proceso de background (Trajectory Miner) analiza los logs de sesiones al final de cada una y extrae:

1. **Pattern Clustering**: identifica patrones recurrentes de error/éxito
2. **Golden Trajectory Extraction**: filtra sesiones con `success_score > 0.85` y extrae la secuencia exacta de tool calls que llevó al éxito
3. **Root Cause Analysis**: para sesiones fallidas, determina si el fallo fue por contexto faltante, limitación del modelo o error procedimental

```rust
pub struct TrajectoryMiner {
    db: tokio_rusqlite::Connection,
    recurring_threshold: u32, // mínimo de sesiones para promover a regla permanente (default: 2)
}

impl TrajectoryMiner {
    pub async fn mine_session(&self, session_id: &str) -> tokio_rusqlite::Result<Vec<ExtractedSkill>> {
        let sid = session_id.to_string();
        let trajectory = self.db.call(move |c| Ok(load_trajectory(c, &sid)?)).await?;

        // 1. SIEMPRE persistir la trayectoria en `episodic_trajectory` — exitosa o no.
        //    Las trayectorias fallidas son insumo para Root Cause Analysis (ver "El ciclo de aprendizaje").
        //    Tabla dedicada: el embedding del task+resolution NO compite contra chat_messages
        //    en el search space (ver doc 6, "Tres tablas — split por granularidad de retrieval").
        let trajectory_id = self.db.call({
            let t = trajectory.clone();
            move |c| Ok(insert_episodic_trajectory(c, &t)?)
        }).await?;

        // 2. Solo Golden Trajectories (success_score > 0.85) califican para extraer skills
        if trajectory.success_score < 0.85 { return Ok(vec![]); }

        let skills = self.extract_tool_sequences(&trajectory);
        // filter_recurring también devuelve los trajectory_ids previos donde la skill apareció
        let recurring = self.filter_recurring(skills).await?;

        // 3. Promoción a `procedural_skill` — INSERT en su PROPIA tabla, NO un UPDATE de
        //    memory_type sobre `memory`. Audit trail vía source_trajectory_ids = JSON
        //    array de los trajectory_ids (incluyendo el actual + los previos).
        for skill in &recurring {
            let s = skill.clone();
            self.db.call(move |c| Ok(promote_to_procedural_skill(c, &s)?)).await?;
        }

        Ok(recurring)
    }
}
```

### El ciclo de aprendizaje

```
Sesión terminada
      ↓
Trajectory Miner analiza logs
      ↓
¿Patrón exitoso? → ¿Ocurrió 2+ veces?
      ↓                    ↓
    No → descarta      Sí → promueve a ProceduralMemory
                            ↓
                    Próxima sesión usa la habilidad codificada
```

### Importance y tier de las skills promovidas

Las skills extraídas por el Trajectory Miner viven en la tabla **`procedural_skill`** (su propia tabla, NO en `memory` con discriminador — ver doc 6, "Tres tablas — split por granularidad de retrieval"). Por convención (cascada de importance del punto 6, reglas de tier del punto 7) nacen ya con:

- **Tabla dedicada `procedural_skill`** — half-life 180 días hardcoded en la query `knn_procedural` (skills validadas son tan estables como decisiones arquitectónicas, ver doc 6 "Búsqueda cross-table"). El tipo es implícito en la tabla; no necesita pasar por `lambda_for_type`.
- `importance = 8.0` — heurística estructural: validadas por `success_score > 0.85` + 2+ ocurrencias justifican alto importance sin necesidad de evaluar con LLM.
- `tier = compute_tier(3, 8.0, 0, "procedural_skill")` → **tier 1** (primary cache) — entran directo a la cache primaria. Por la función pura `compute_tier` (punto 6), el label `procedural_skill` resuelve a tier 1 estructural: ya pasaron una barra de validación alta, no tiene sentido hacerlas competir desde el archive.
- **Audit trail vía `source_trajectory_ids`** — JSON array de los `episodic_trajectory.id` que dieron origen a la skill. Permite reconstruir "¿de dónde salió esta skill?" en debugging y respeta el principio de explicabilidad del punto 13.

`promote_to_procedural_skill` debe invocar `compute_tier` al setear el tier inicial, no hardcodear el valor — así si más adelante cambian las reglas, el cambio vive en un solo lugar (`src-tauri/src/memory/tier.rs`). La función pura `compute_tier` recibe el label `"procedural_skill"` como `type_label` (parámetro genérico que cubre tanto `memory_type` de la tabla `memory` como nombre de tabla dedicada).

### Criterio de éxito

- Después de la tercera sesión resolviendo el mismo tipo de problema, el agente lo resuelve en menos pasos
- Los errores recurrentes (mismas categorías de fallo) disminuyen progresivamente con el tiempo

---

## Next Steps

- 9. autoDream — Consolidación en Background
- 10. Strategic Forgetting — Olvido Intencional
- 11. Privacidad y Cifrado en Reposo
- 12. Multi-tenancy de Memorias — Scopes y Contextos
- 13. Lifecycle de Memorias — Borrado, Auditoría y Transferencia