import List from "std/list"

export List.map

extern "libm"
  def cos(x: C.Double) -> C.Double
  compute def pow(x: C.Double, out y: C.Double) -> C.Double = "pow"
  callback Compare(a: C.Pointer, b: C.Pointer) -> C.Int
  resource Handle, free: closehandle
  struct Timeval
    seconds: C.Long
  end
end

def first[T](items: List[T]) -> T
  return List.first(items)
end

extern module "sqlite"
  resource Db
  def open(path: String) -> Result[Db]
  compute def save!(db: Db) -> Result[Void]
end
