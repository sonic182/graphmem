# graphmem retrieval — medir calidad, no solo comportamiento

Hoy no hay forma de saber si un cambio de retrieval (tocar un default en
`[retrieval]`, o más adelante CatRAG vs HippoRAG 2, ver `graph_tune.md`)
mejora o empeora los resultados en general. Solo hay tests puntuales.

## Lo que existe hoy

- `src/retrieval_eval.rs`: fixture de ~9 memories con un embedder
  determinista (`BagOfWords`, no el modelo real) y `assert_eq!`/`assert!`
  sobre casos concretos (p.ej. `ranks_the_memory_that_matches_the_whole_query_first`,
  `the_graph_channel_weight_moves_score_toward_linked_memories`).
- `sweeps_the_memory_channel_weight` (`#[ignore]`): imprime una tabla para
  inspección manual, no calcula ninguna métrica agregada.

Esto verifica *comportamiento* puntual (no romper un caso conocido al
tocar un knob), no *calidad* agregada.

## Lo que falta

- [ ] Set fijo de `(query, memory_esperada)` más grande que los ~9 casos
      actuales (mismo patrón de fixture, más entradas)
- [ ] Métrica agregada tipo `recall@k` y/o `MRR` sobre ese set, no asserts
      sueltos por caso
- [ ] Reemplazar (o complementar) `sweeps_the_memory_channel_weight` con
      una versión que imprima la métrica agregada por valor del knob, no
      solo el ranking crudo
- [ ] Mecanismo para comparar dos configuraciones/algoritmos lado a lado
      (hoy HippoRAG 2 vs HippoRAG 2 con otro `damping`; más adelante,
      HippoRAG 2 vs CatRAG cuando exista Fase 2) sobre el mismo set
- [ ] Decidir si el set de eval sigue usando `BagOfWords` (rápido,
      determinista, ya usado) o el embedder real (más realista, más lento,
      no determinista salvo fijar semilla/modelo)

No implica infraestructura nueva (nada de dataset externo tipo STaRK por
ahora) — es extender `retrieval_eval.rs` con el mismo patrón que ya usa.
