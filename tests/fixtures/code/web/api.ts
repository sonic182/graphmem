export interface Order {
  id: string;
}

export async function fetchOrder(id: string): Promise<Order> {
  return { id };
}

import type { Client } from "./client";

import fs = require("fs-extra");
export type { Order as Row } from "./rows";
