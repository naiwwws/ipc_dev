import { BL410ModbusReader } from '../src/modbus';

async function testConnection() {
  const reader = new BL410ModbusReader();
  
  // Configuration - adjust these values for your setup
  const port = '/dev/ttyS0';  // Change to your port
  const baudRate = 9600;        // Common: 9600, 19200, 38400, 115200
  const deviceAddress = 2;      // Your flowmeter device address
  
  try {
    console.log('🧪 Testing Modbus connection...\n');
    
    // Connect to RS485
    await reader.connect(port, baudRate, 'even');
    
    // Test reading a single flowmeter
    console.log(`📊 Attempting to read flowmeter at address ${deviceAddress}...\n`);
    
    const data = await reader.readFlowmeterData(deviceAddress);
    
    if (data) {
      console.log('✅ Successfully read flowmeter data:');
      console.log('='.repeat(40));
      console.log(`Device Address: ${data.deviceAddress}`);
      console.log(`Timestamp: ${data.timestamp.toISOString()}`);
      console.log(`Error Code: ${data.errorCode}`);
      console.log(`Mass Flow Rate: ${data.massFlowRate.toFixed(2)} kg/h`);
      console.log(`Density: ${data.densityFlow.toFixed(4)} kg/m³`);
      console.log(`Temperature: ${data.temperature.toFixed(2)} °C`);
      console.log(`Volume Flow Rate: ${data.volumeFlowRate.toFixed(3)} m³/h`);
      console.log(`Mass Total: ${data.massTotal.toFixed(2)} kg`);
      console.log(`Volume Total: ${data.volumeTotal.toFixed(3)} m³`);
    } else {
      console.log('❌ Failed to read flowmeter data');
      console.log('💡 Check:');
      console.log('   • Device address is correct');
      console.log('   • Wiring connections');
      console.log('   • Baud rate settings');
      console.log('   • Device power');
    }
    
  } catch (error) {
    console.error('❌ Connection test failed:', error);
    console.log('\n💡 Troubleshooting tips:');
    console.log('   • Check if the serial port exists and has proper permissions');
    console.log('   • Verify RS485 wiring (A, B, GND)');
    console.log('   • Ensure correct baud rate and parity settings');
    console.log('   • Check if another application is using the port');
  } finally {
    reader.disconnect();
  }
}

testConnection();
