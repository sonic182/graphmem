import List from "std/list"

export List.map

extern "libm"
  def cos(x: C.Double) -> C.Double
  export compute def pow(x: C.Double, out y: C.Double) -> C.Double = "pow"
  callback Compare(a: C.Pointer, b: C.Pointer) -> C.Int
  resource Handle, free: closehandle
  export struct Timeval
    seconds: C.Long
  end
end

def first[T](items: List[T]) -> T
  return List.first(items)
end
