export type FillPort = {
  postMessage(message: unknown): void;
  disconnect(): void;
  onMessage: {addListener(callback: (message: unknown) => void): void};
  onDisconnect: {addListener(callback: () => void): void};
};

/** Keep the popup connection alive until the worker accepts the selection.
 * Closing the popup then releases the worker's native-unlock barrier. */
export function queueFill(connect: () => FillPort, message: unknown): Promise<boolean> {
  return new Promise((resolve, reject) => {
    const port = connect();
    let settled = false;
    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      port.disconnect();
      reject(new Error("Fill handoff timed out."));
    }, 15000);
    port.onDisconnect.addListener(() => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      reject(new Error("The extension background connection closed."));
    });
    port.onMessage.addListener(reply => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      const accepted = !!reply && typeof reply === "object" && "ok" in reply && reply.ok === true;
      if (!accepted) port.disconnect();
      resolve(accepted);
    });
    try { port.postMessage(message); }
    catch (error) {
      settled = true;
      clearTimeout(timer);
      port.disconnect();
      reject(error);
    }
  });
}
