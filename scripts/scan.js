
// Scan and connect/enumerate devices

const async_scan = async (args = {}) => {
  // Get first matching instance from scanner
  const scanner = scan({ filter_seen: true, ...args });
  for await (const dev of scanner) {
      dev.connect().then(async () => {
        await dev.enumerate();
        console.log(JSON.stringify(await dev.snapshot(),null,2))
        await dev.disconnect();
      })
  }
}
