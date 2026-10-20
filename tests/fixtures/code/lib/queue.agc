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
