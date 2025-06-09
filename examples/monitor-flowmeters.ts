import { BL410ModbusReader } from '../src/modbus';

async function monitorFlowmeters() {
  const reader = new BL410ModbusReader();
  
  // Configuration
  const config = {
    port: '/dev/ttyUSB0',
    baudRate: 9600,
    parity: 'none' as const,
    deviceAddresses: [1, 2, 3, 4], // Adjust to your device addresses
    intervalMs: 5000 // 5 seconds
  };
  
  try {
    console.log('🖥️  Vessel Flowmeter Monitor\n');
    
    // Connect to RS485
    await reader.connect(config.port, config.baudRate, config.parity);
    
    // Start monitoring
    await reader.monitorFlowmeters(config.deviceAddresses, config.intervalMs);
    
  } catch (error) {
    console.error('❌ Monitor failed:', error);
    reader.disconnect();
    process.exit(1);
  }
}

monitorFlowmeters();
