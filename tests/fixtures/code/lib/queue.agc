import "std/list" as List

export struct QueueState
  items: List[String]
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
