
const CH9143_NOTIFY_UUID = "0xfff1::utf8";
const CH9143_WRITE_UUID = "0xfff2::utf8";

const ch9143  = async ({showServices = false} = {}) => {
  // Get first matching instance from scanner
  const scanner = scan({name: "CH9143BLE2U"});
  const dev = (await scanner.next()).value;

  // Close scanner
  scanner.close();

  // Connect to device & enumerate
  console.log(">> CONNECT");
  await dev.connect();
  console.log(">> ENUMERATE");
  await dev.enumerate();
  if (showServices) {
    console.log(JSON.stringify(await dev.snapshot(),null,2))
  }

  // Subscribe to notify characteristic
  await dev.subscribe([CH9143_NOTIFY_UUID], true);

  // Install notification handler
  const n = dev.on_notification((n) => {
    __print(n.value[0])
  });

  while (true) {
    const line = await __readline_async();
    if (line === undefined) {
      break;
    }
    await dev.write(CH9143_WRITE_UUID, line + "\r\n", true);
  }

  return () => { n.stop(); dev.disconnect() };
}

