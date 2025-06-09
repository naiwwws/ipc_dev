import { SerialPort } from 'serialport';

async function listPorts() {
  try {
    console.log('🔍 Scanning for available serial ports...\n');
    const ports = await SerialPort.list();
    
    if (ports.length === 0) {
      console.log('❌ No serial ports found');
      return;
    }

    console.log('Available serial ports:');
    console.log('='.repeat(50));
    
    ports.forEach((port, index) => {
      console.log(`${index + 1}. ${port.path}`);
      console.log(`   Manufacturer: ${port.manufacturer || 'Unknown'}`);
      console.log(`   Serial Number: ${port.serialNumber || 'N/A'}`);
      console.log(`   Vendor ID: ${port.vendorId || 'N/A'}`);
      console.log(`   Product ID: ${port.productId || 'N/A'}`);
      console.log('');
    });
    
    // Suggest likely RS485 ports
    const rs485Ports = ports.filter(port => 
      port.manufacturer?.toLowerCase().includes('ftdi') ||
      port.manufacturer?.toLowerCase().includes('prolific') ||
      port.manufacturer?.toLowerCase().includes('ch340') ||
      port.path.includes('USB')
    );
    
    if (rs485Ports.length > 0) {
      console.log('🎯 Likely RS485 adapter ports:');
      rs485Ports.forEach(port => {
        console.log(`   • ${port.path} (${port.manufacturer})`);
      });
    }
    
  } catch (error) {
    console.error('❌ Error listing ports:', error);
  }
}

listPorts();
