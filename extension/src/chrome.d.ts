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
        sendNativeMessage(host: string, message: unknown): Promise<unknown>;
      };
    }
  | undefined;
