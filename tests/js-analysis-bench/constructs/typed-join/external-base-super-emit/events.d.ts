declare module "events" {
  export class EventEmitter {
    emit(event: string, ...args: unknown[]): boolean;
    on(event: string, listener: (...args: any[]) => void): this;
  }
}
