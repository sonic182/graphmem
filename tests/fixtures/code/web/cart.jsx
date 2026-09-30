import React from "react";

export function Cart({ items }) {
  return <ul>{items.map((item) => <li key={item.id}>{item.name}</li>)}</ul>;
}

export class CartStore {
  add(item) {
    this.items.push(item);
  }
}

export default {
  mounted() {
    this.el.focus();
  },
  updated: () => {},
};
