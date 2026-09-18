# graphmem retrieval — roadmap hacia CatRAG

Orden conceptual: **Fase 1, HippoRAG 2 (base, implementada) → Fase 2, CatRAG
(dynamic edge weighting, objetivo actual) → Fase 3, GraphFlow (retrieval
aprendido, techo futuro, no planificado)**.

---

## Fase 1 — HippoRAG 2: grafo + PPR como base

**Paper:** *From RAG to Memory: Non-Parametric Continual Learning for Large Language Models*, ICML 2025. ([Paper ICML/PMLR][1]) · [Código oficial][hippocode]

- [x] Passage/memory nodes dentro del grafo
- [x] Embeddings de memories (`Qwen/Qwen3-Embedding-0.6B` en V1; ver Fase 2.5 / PR #2 para el switch de modelo)
- [x] Enlaces explícitos memoria-entidad
- [x] PPR bidireccional con damping `0.5`
- [x] Filtro de scope aplicado antes de construir los nodos de memoria retornables

Un vector search encuentra cosas semánticamente parecidas pero puede perder
una cadena de relaciones. HippoRAG 2 mete los documentos y sus conceptos en
un mismo grafo y usa Personalized PageRank (PPR) para propagar relevancia a
través de relaciones multi-hop, evitando que esa mejora multi-hop destruya
el recall de preguntas simples. ([Paper ICML/PMLR][1])

El grafo tiene dos clases de nodos:

```text
Phrase / Entity ──relation──▶ Phrase / Entity
Phrase / Entity ──context───▶ Passage
```

En indexing, un LLM hace OpenIE (`"Johanderson desarrolla Minibot en
Python"` → `(Johanderson, develops, Minibot)`, `(Minibot, uses, Python)`);
los términos se convierten en phrase nodes y el passage original permanece
como nodo. Se añaden relation edges, synonym edges entre conceptos
similares y context edges entre conceptos y passages. ([OpenReview][2])

En query time: primero se hace embedding de la query contra los **triples
completos**, no solo contra nombres de entidad. Luego un LLM hace
**Recognition Memory**: filtra esos triples y deja los relacionados con la
intención de la pregunta; los conceptos supervivientes se convierten en
seeds fuertes del PPR. ([Memory Papers][3]) Hay una segunda señal —
similitud directa query↔passage— mezclada con un factor pequeño
(`λ≈0.05`) para no eclipsar el grafo. Damping `0.5`. ([Código oficial][4])

```text
π(t+1) = (1-d)·seed + d·Tᵀ·π(t)
score(passage) = PPR[passage]
```

No requiere entrenar ningún modelo — por eso fue la primera base razonable
para graphmem.

---

## Fase 1.1 — corrección de scoring

- [x] Vector de personalización con dos canales normalizados por separado (memoria + grafo)
- [x] Softmax de temperatura `0.05` para memorias (top-k por coseno) — restaura el contraste que la banda estrecha de los embeddings destruía
- [x] Symbolic anchoring: ancla aditiva `ε` (`entity_anchor_weight`) para entidades nombradas literalmente en la query, vía `text_mentions`
- [x] Embeddings propios de entidades — una paráfrasis puede anclar una entidad sin necesitar edges
- [x] Mezcla memoria/grafo controlada por `memory_seed_weight`
- [x] PPR lineal: la masa dangling se acumula en un escalar en vez de repartirse nodo a nodo
- [x] Constantes de retrieval movidas a `[retrieval]` en `config.toml`
- [x] `src/retrieval_eval.rs` fija el comportamiento con un embedder determinista para barrer valores

Esta fase corrigió la parte que no se comportaba como el paper (el scoring),
sin tocar aún la forma del grafo ni las transiciones del PPR. Dejó abierto,
a propósito, el hueco de `edge_weights` por query — eso es exactamente
Fase 2.

---

## Fase 2 — CatRAG: dynamic edge weighting (objetivo actual)

**Paper:** *Breaking the Static Graph: Context-Aware Traversal for Graph-Based RAG*, Findings ACL 2026. ([Paper ACL][5]) · [Repositorio CatRAG][6]

CatRAG parte de HippoRAG 2 y señala un defecto: la query cambia el seed
vector, pero los pesos de las edges del grafo se quedan fijos (el
**"Static Graph Fallacy"**). `Johanderson --develops--> Minibot`,
`--likes--> Python`, `--lives--> Madrid` tienen el mismo peso estructural
sin importar si preguntas por software, ciudades o coches.

### Estado actual frente al CatRAG oficial

(Contrastado contra `src/catrag/CatRAG.py`, publicado el 20 de agosto.)

- [x] Passage/memory nodes dentro del grafo
- [x] Embeddings de memories, entidades y triples/edges
- [x] PPR con `damping=0.5`
- [x] Seed mixture memory + graph
- [x] Symbolic anchoring (equivalente al de CatRAG, sin NER vía LLM)
- [x] Query→fact/edge similarity (`edge_vector` + `edge_document()` = `"source relation target"`)
- [ ] Coarse edge filtering — 🟡 infraestructura lista (`edge_vector`), falta activarla como filtro local en vez de global
- [ ] Query-specific edge weights
- [ ] Weighted/directed PPR
- [ ] Key-Fact Passage Enhancement (provenance `memory_edges` + boost `×2.5`)
- [ ] Cross-encoder edge scorer (reranker local, MiniLM) — planeado, reemplaza al `LLM_classify` del paper
- [ ] Recognition Memory (filtro LLM de triples) — diferido a propósito, fuera de este roadmap

Estimación: ~65–70% de un "CatRAG-lite" razonable para graphmem; ~45–50% de
una reproducción fiel del paper completo (que además exige un LLM scorer).

`semantic_results()` ya calcula casi todas las señales que hacen falta:
embedding de query, scores de memories, scores de entities y scores de
edges usando el triple completo, convirtiendo los mejores edges en seeds y
ejecutando PPR. El salto que falta es conceptualmente pequeño: que la señal
`query↔edge` alimente las **transiciones** del random walk, no solo el
**restart vector**. Hoy `personalized_pagerank` recibe una adyacencia plana
(`adjacency: &[Vec<usize>]`) y reparte `rank[node] * damping /
neighbors.len()` por igual entre vecinos — ahí está literalmente el
Static Graph Fallacy.

Los tres mecanismos de CatRAG, y cómo mapean a graphmem:

1. **Symbolic Anchoring** — ✅ ya hecho (`entity_anchor_weight` + `text_mentions`, sin llamada a LLM para NER).
2. **Query-Aware Dynamic Edge Weighting** — filtro barato por coseno sobre las outgoing edges de los top seeds (`Nseed=5`, `Kedge=15` en el paper oficial), y solo entonces (en el paper) un LLM clasifica cada edge en `Irrelevant/Weak/High/Direct` → multiplicador de peso. graphmem sustituye ese LLM por un **cross-encoder reranker local tipo MiniLM** (p.ej. `cross-encoder/ms-marco-MiniLM-L-6-v2`): mismo rol de juez query↔fact, pero un modelo pequeño BERT-family, cargable con `candle_transformers::models::bert` igual que DistilBERT hoy en `embedding.rs`, sin API externa. Da un score continuo en vez de 4 etiquetas, que se mapea a multiplicador con una función monótona — se verifica con `retrieval_eval.rs`, no con un juez externo.
3. **Key-Fact Passage Enhancement** — si un triple es relevante, los passages que lo contienen reciben `new_weight(entity, passage) = weight * (1 + β)` con `β=2.5` en el paper. Requiere saber qué facts aparecen en qué passage — hoy graphmem no guarda esa relación.

```text
                 original                          query-aware
Minibot ─uses──────── Python   w=1        Minibot ─uses──────── Python   w=8
       ─created_by── Johanderson w=1             ─created_by── Johanderson w=.2
       ─hosted_on─── Debian     w=1             ─hosted_on─── Debian     w=.5
```

### Subfases de implementación (mini-entregables, sin LLM ni llamadas externas)

Fase 2 se corta en cuatro PRs pequeños. `2a`+`2b` son el núcleo de CatRAG;
`2c` y `2d` son mejoras independientes que se montan encima. Cada una deja el
retrieval funcional por sí sola.

#### Fase 2a — plumbing del overlay de pesos (PR mínimo)

- [ ] `personalized_pagerank` acepta un `edge_weight_overlay: Option<&HashMap<(usize, usize), f64>>` opcional — vacío = comportamiento actual (Fase 1/1.1 intacta, cambio aditivo).
- [ ] Test en `domain.rs` que verifica que un overlay vacío reproduce exactamente el ranking de la firma antigua.

Valor: aisla el cambio de firma de `domain.rs` del scoring nuevo. Sin overlay no
hay comportamiento nuevo, así que no rompe Fase 1/1.1.

#### Fase 2b — dynamic edge weights desde los seeds (núcleo CatRAG)

- [ ] Para los top seed entities, tomar solo sus outgoing edges y reutilizar el score `query↔edge_vector` que ya se calcula hoy (hoy se usa globalmente para elegir seeds; aquí se usa localmente para ponderar transiciones); mapear coseno → multiplicador con una función monótona simple.
- [ ] Alimentar ese overlay a las transiciones de PPR vía el parámetro de `2a`: que la señal `query↔edge` mueva las transiciones del random walk, no solo el restart vector.
- [ ] Activar el coarse edge filtering como filtro **local** (top seeds × `K_edge`), no global.
- [ ] Nuevo fixture "hub semantic drift" en `retrieval_eval.rs` (una entidad con muchos edges de temas distintos) para verificar que el reponderado reduce la dispersión hacia vecinos irrelevantes, no solo mejora casos ya fáciles.

Valor: es el salto conceptual — elimina literalmente la Static Graph Fallacy.
Toda la señal ya se calcula; aquí solo cambia dónde se consume.

#### Fase 2c — provenance y Key-Fact Passage Enhancement

- [ ] Tabla de provenance:
  ```sql
  CREATE TABLE memory_edges (
      memory_id INTEGER NOT NULL,
      edge_id   INTEGER NOT NULL,
      PRIMARY KEY (memory_id, edge_id)
  );
  ```
  sin duplicar texto ni atributos; `remember_with_graph` la puebla al crear los edges de una memory.
- [ ] Migración de esquema para stores existentes.
- [ ] Key-Fact Passage Enhancement: cuando un edge es relevante, boost `×2.5` de la transición entity→memory correspondiente vía `memory_edges`, renormalizando la masa total.

Valor: separable del scoring (es migración + provenance). Requiere `2a` para
boostear transiciones, pero no `2b`.

#### Fase 2d — cross-encoder edge scorer (calidad opcional)

- [ ] Trait `EdgeScorer` para desacoplar "cómo se puntúa un edge candidato" de PPR, con tres niveles detrás de la misma interfaz:
  ```rust
  trait EdgeScorer {
      fn score(&self, query: &str, candidates: &[EdgeCandidate]) -> Vec<f64>;
  }
  ```
  - `EmbeddingEdgeScorer` (usa directamente el coseno del filtro coarse + mapeo monótono, ya disponible como señal hoy) — default, coste cero adicional.
  - `CrossEncoderEdgeScorer` (re-puntúa el mismo shortlist con el reranker MiniLM local, ver mecanismo 2 arriba) — mejora de calidad opcional vía config, sigue siendo 100% local/sin API. Cargar un cross-encoder MiniLM (p.ej. `cross-encoder/ms-marco-MiniLM-L-6-v2`, BERT-family, vía `candle_transformers::models::bert`) que puntúe `(query, edge_document(edge))` como par conjunto en vez de comparar embeddings independientes — mismo patrón "retrieve con bi-encoder, rerank con cross-encoder" que ya usa search bi-encoder + este reranker.
  - `LlmEdgeScorer` (LLM externo tipo el paper original) — posibilidad futura detrás de la misma interfaz, **no implementarlo todavía**.
- [ ] Config `[retrieval] edge_scorer = "cosine" | "cross_encoder"` (default `"cosine"`), mismo patrón que `[embedding] model`/`enabled` (`config.toml` + override por env var). No es una elección mutuamente excluyente de motor: el filtro coarse por coseno **siempre** corre primero (selecciona los `K_edge` candidatos reutilizando embeddings ya calculados); el config solo decide si además se re-puntúa ese shortlist con el cross-encoder antes de convertirlo en multiplicador. `"cosine"` es más liviano (coste ~0, ya calculado); `"cross_encoder"` añade una forward pass por candidato (~`K_edge`≈15 por query) a cambio de mayor precisión.

Valor: el más caro y el más aislable. El trait espera a que exista el segundo
scorer real; no se introduce antes (YAGNI).

La API objetivo es la misma que ya se imaginó desde el principio:

```python
graph.pagerank(
    seeds={...},
    edge_weights={...},   # overlay opcional, específico de la query
)
```

`edge_weights` vacío → Fase 1 (HippoRAG 2). `edge_weights =
score_edges(query, candidate_edges)` → núcleo de CatRAG.

### Fase 2.5 — prerrequisito ya mergeado (PR #2)

- [x] Switch de embeddings a `sentence-transformers/msmarco-distilbert-cos-v5`
- [x] `gmem reembed` como comando de migración explícita
- [x] Truncation a `max_position_embeddings` del modelo (512 tokens por defecto)
- [x] `reembed` resiliente a fallos por item (no aborta el store entero por un registro)

No avanza el algoritmo de retrieval, pero deja memories/entities/edges con
embeddings consistentes bajo un único modelo, remigrable explícitamente —
más seguro para experimentar con dynamic edge weighting encima.

---

## Fase 3 — GraphFlow: retrieval aprendido (techo futuro, no planificado)

**Paper:** *Can Knowledge-Graph-based Retrieval Augmented Generation Really Retrieve What You Need?*, NeurIPS 2025 Spotlight. ([Paper NeurIPS][7]) · [Código GraphFlow][graphflowcode]

- [ ] Trajectory sampler (retrieval como secuencia de decisiones, no solo scores + PPR)
- [ ] Policy model (LLM backbone compartido + adapters LoRA + policy MLP)
- [ ] Flow estimator (`log F(state)`, consistencia tipo GFlowNet / detailed balance)
- [ ] Acción de self-loop (`node → itself`) para aprender cuándo parar en vez de fijar `max_hops`
- [ ] Local exploration durante training (muestrear vecinos alternativos, no solo la ruta ground-truth)
- [ ] Dataset de entrenamiento con trayectorias etiquetadas

HippoRAG/CatRAG son "calcula scores → ejecuta PPR". GraphFlow trata el
retrieval como una secuencia de decisiones (`Johanderson → Minibot → Python
→ asyncio`) entrenada con una idea de GFlowNets: aprende simultáneamente una
policy y una función de flujo `F(state)` que permite propagar la recompensa
final hacia decisiones intermedias sin etiquetar manualmente cada paso.
Como el traversal no retrocede, la backward policy se simplifica a `1`.

En inferencia se muestrean varias trayectorias en paralelo; al asignar
probabilidad proporcional a la recompensa (en vez de colapsar al máximo),
tiende a producir varios resultados buenos distintos — mejoras fuertes en
recall y deduplicated recall en STaRK (~+10% medio frente a baselines
fuertes). ([Paper NeurIPS][7])

Para graphmem esto ya implicaría entrenamiento (policy + flow model +
dataset), no solo una función de scoring:

```text
GraphMem
  ├── graph store
  ├── vector embeddings
  ├── query states
  ├── trajectory sampler
  ├── policy model
  ├── flow estimator
  └── training dataset
```

No se implementaría como primer algoritmo del proyecto — es el techo de la
hoja de ruta, no un objetivo con fecha.

---

[1]: https://proceedings.mlr.press/v267/gutierrez25a.html "From RAG to Memory: Non-Parametric Continual Learning for Large Language Models"
[hippocode]: https://github.com/OSU-NLP-Group/HippoRAG "Código oficial HippoRAG"
[2]: https://openreview.net/pdf?id=LWH8yn4HS2 "From RAG to Memory: Non-Parametric Continual Learning for Large Language Models"
[3]: https://memorypapers.org/papers/hipporag-2-rag-to-memory "From RAG to Memory: Non-Parametric Continual Learning for Large Language Models | Memory Papers"
[4]: https://github.com/lucagattoni/Pinakes/blob/main/docs/graph/hipporag.md "pinakes/docs/graph/hipporag.md at main · lucagattoni/pinakes · GitHub"
[5]: https://aclanthology.org/2026.findings-acl.290/ "Breaking the Static Graph: Context-Aware Traversal for Graph-Based RAG - ACL Anthology"
[6]: https://github.com/kwunhang/CatRAG "GitHub - kwunhang/CatRAG"
[7]: https://proceedings.neurips.cc/paper_files/paper/2025/hash/89d0d5c2f720921df93bbb8fef514571-Abstract-Conference.html "Can Knowledge-Graph-based Retrieval Augmented Generation Really Retrieve What You Need?"
[graphflowcode]: https://github.com/Samyu0304/GraphFlow "Código GraphFlow"
