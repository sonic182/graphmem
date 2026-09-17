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

- [x] Set fijo de `(query, memory_esperada)` más grande que los ~9 casos
      actuales — cubierto fuera de Rust con `scripts/eval_retrieval.py`
      sobre HotpotQA, 2WikiMultihopQA y MuSiQue (`download_eval_datasets.py`)
- [x] Métrica agregada tipo `recall@k` y/o `MRR` sobre ese set
- [x] Grafo para evaluar la parte HippoRAG sin LLM:
      `scripts/extract_eval_graphs.py` extrae entidades y tripletas SVO con
      spaCy (`en_core_web_sm`) para todos los párrafos por igual
      (`--graphs spacy`). `oracle` (tripletas del dataset) solo sirve como
      techo: cubre únicamente el camino de la respuesta
- [x] Corpus compartido (`--corpus shared`, por defecto): todos los párrafos
      en un scope, como evalúa HippoRAG; por pregunta recall@10 ≈ 1
- [x] Decidir `BagOfWords` vs embedder real: el script usa el real
      (`msmarco-distilbert-cos-v5`, CPU, batch 1 por defecto)
- [ ] Reemplazar (o complementar) `sweeps_the_memory_channel_weight` con
      una versión que imprima la métrica agregada por valor del knob, no
      solo el ranking crudo — en el script: pasar `GRAPHMEM_RETRIEVAL_*`
      y añadir una columna por configuración
- [ ] Mecanismo para comparar dos configuraciones/algoritmos lado a lado
      (hoy HippoRAG 2 vs HippoRAG 2 con otro `damping`; más adelante,
      HippoRAG 2 vs CatRAG cuando exista Fase 2) sobre el mismo set
- [ ] Guardar resultados (JSON con commit, modelo, config y métricas) para
      comparar entre commits
- [ ] Reutilizar los vectores de las memorias entre grafos: hoy cada
      combinación vuelve a embeberlas (~0.4 s/párrafo en CPU)
- [ ] Relaciones de spaCy escasas (<1 por párrafo): valorar patrones
      extra (aposiciones, "X is a Y") o un modelo mayor (`--model`)
