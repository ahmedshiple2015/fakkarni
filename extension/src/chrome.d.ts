interface FakkarniTab {
  id?: number;
  url?: string;
}
declare const chrome:
  | {
      tabs: {
        query(query: {
          active: boolean;
          currentWindow: boolean;
        }): Promise<FakkarniTab[]>;
      };
      runtime: {
        connect(options: {name: string}): import("./handoff.js").FillPort;
        sendNativeMessage(host: string, message: unknown): Promise<unknown>;
        sendMessage(message: unknown): Promise<unknown>;
      };
    }
  | undefined;
