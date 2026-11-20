import List from "std/list"
import {append, Item as Entry} from "std/items"

LIMIT = 10

type Items = List[String]

struct QueueState
  items: Items
end

enum Shape
  Circle(Float)
  Empty
end

def push(q: QueueState, value: String) -> QueueState
  return QueueState(items: List.append(q.items, value))
end

def size(q: QueueState) -> Int
  return List.size(q.items)
end

def describe(shape: Shape, limit: Int = 1_000) -> String
  label = <<~TEXT
    shape up to #{limit}
  TEXT
  case shape
    Shape.Circle(0.5) -> "unit"
    Shape.Circle(r) if r > 0.000_5 -> label
    Shape.Empty -> "empty"
  end
  return label
end

struct Page[T]
  rows: List[T]
  size: Int = 0xFF
end

def pattern[T](value: T) -> Regex
  scale = 1.5e-3
  return /queue\/[a-z]+/i
end

## Renders pages; `#{` in a raw heredoc stays text.
module Templates
  def page(title: String) -> String
    return <<~EAG + title
      <h1>#{ b: 1 } \q</h1>
    EAG
  end
end

private

def limit_of(limit: Int = LIMIT) -> Int = limit

def row_count(rows: List[Storage.Tables.Row]) -> Int = List.size(rows)
