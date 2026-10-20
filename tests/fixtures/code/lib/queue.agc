import List from "std/list"
import {append, Item as Entry} from "std/items"

LIMIT = 10

type Items = List[String]

export struct QueueState
  items: Items
end

enum Shape
  Circle(Float)
  Empty
end

export def push(q: QueueState, value: String) -> QueueState
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
