import { createServer } from "node:net";

// A port the OS just let us bind: a pid-derived one can sit in a Windows excluded range (Hyper-V/WinNAT reservations on the
// runners), where the browser cannot listen and says so only in its own log.
export const freePort = () =>
  new Promise((ok, no) => {
    const srv = createServer();
    srv.once("error", no);
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => ok(port));
    });
  });
