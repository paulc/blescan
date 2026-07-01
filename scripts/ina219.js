
// Connect to an INA219 BLE power-meter (https://github.com/paulc/power-meter)
// and subscribe to updates

const ina219 = async ({json = false, showServices = false} = {}) => {
  // Get first matching instance from scanner
  const scanner = scan({name:"INA219"});
  const dev = (await scanner.next()).value;
  // Close scanner
  scanner.close();
  // Connect to device & enumerate
  await dev.connect();
  await dev.enumerate();
  if (showServices) {
    console.log(JSON.stringify(await dev.snapshot(),null,2))
  }
  // Write to timer characteristic - update NOTIFY timer to 1s
  await dev.write("00000005-9b04-4347-98ff-57e8f7803509::u32",1);
  // Subscribe to V/I updates
  await dev.subscribe(["00000003-9b04-4347-98ff-57e8f7803509::f32,f32"]);
  // Install notification handler
  const n = dev.on_notification((n) => {
    const now = new Date().toISOString();
    if (json) {
      const data = { 
        time: now,
        v: n.value[0],
        i: n.value[1],
      };
      console.log(JSON.stringify(data));
    } else {
      console.log(`Time: ${now} Vbus: ${n.value[0].toFixed(3).padStart(6)}V Ishunt: ${n.value[1].toFixed(3).padStart(6)}mA Power: ${(n.value[0] * n.value[1]).toFixed(3).padStart(6)}mW`)
    }
  });
  // Return stop function - cancel subscription & disconnect
  return () => { n.stop(); dev.disconnect() };
}

