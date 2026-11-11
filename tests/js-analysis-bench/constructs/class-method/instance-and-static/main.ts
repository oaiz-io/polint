// @exports
class Queue {
  private items: number[];

  constructor() { // @fn Queue.constructor
    this.items = [];
  }

  static of(first: number): Queue { // @fn Queue.of
    const queue = new Queue(); // @calls Queue.constructor
    queue.push(first); // @calls Queue.push
    return queue;
  }

  push(item: number): number { // @fn Queue.push
    this.items.push(item); // @external
    return this.size(); // @calls Queue.size
  }

  size(): number { // @fn Queue.size
    return this.items.length;
  }
}

Queue.of(1).push(2); // @calls Queue.of Queue.push
