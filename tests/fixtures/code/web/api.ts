export interface Order {
  id: string;
}

export async function fetchOrder(id: string): Promise<Order> {
  return { id };
}
