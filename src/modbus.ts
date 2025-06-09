import ModbusRTU from 'modbus-serial';
import { SerialPort } from 'serialport';

interface ModbusDevice {
  id: number;
  name: string;
  registers: {
    address: number;
    count: number;
    type: 'holding' | 'input' | 'coil' | 'discrete';
    description: string;
  }[];
}

interface FlowmeterData {
  deviceAddress: number;
  timestamp: Date;
  errorCode: number;
  massFlowRate: number;
  densityFlow: number;
  temperature: number;
  volumeFlowRate: number;
  massTotal: number;
  volumeTotal: number;
  massInventory: number;
  volumeInventory: number;
}

class BL410ModbusReader {
  private client: ModbusRTU;
  private isConnected: boolean = false;
  private devices: ModbusDevice[] = [];
  private flowmeterRegOffset: number = 1;

  constructor() {
    this.client = new ModbusRTU();
  }

  /**
   * Initialize RS485 connection
   * @param port Serial port (e.g., '/dev/ttyS0' for BL410)
   * @param baudRate Baud rate (default: 9600)
   * @param parity Parity setting (default: 'none')
   */
  async connect(port: string = '/dev/ttyS0', baudRate: number = 9600, parity: 'none' | 'even' | 'odd' = 'none'): Promise<void> {
    try {
      console.log(`Connecting to RS485 port: ${port}`);
      console.log(`Baud rate: ${baudRate}, Parity: ${parity}`);

      await this.client.connectRTUBuffered(port, {
        baudRate: baudRate,
        dataBits: 8,
        stopBits: 1,
        parity: parity,
      });

      // Set timeout for Modbus operations
      this.client.setTimeout(1000);

      this.isConnected = true;
      console.log('✅ RS485 Modbus connection established');
    } catch (error) {
      console.error('❌ Failed to connect to RS485:', error);
      throw error;
    }
  }

  /**
   * Convert two 16-bit registers to a 32-bit integer (Big Endian)
   * Mimics the Lua bit manipulation: (arg[i] << 8 | arg[i+1]) << 16 | (arg[i+2] << 8 | arg[i+3])
   */
  private registers32ToInt(registers: number[], startIndex: number): number {
    const high16 = (registers[startIndex] << 8) | registers[startIndex + 1];
    const low16 = (registers[startIndex + 2] << 8) | registers[startIndex + 3];
    return (high16 << 16) | low16;
  }

  /**
   * Convert 32-bit integer to IEEE 754 float
   * Mimics the Lua string.unpack("f", string.pack("i4", value))
   */
  private int32ToFloat(value: number): number {
    const buffer = new ArrayBuffer(4);
    const intView = new Int32Array(buffer);
    const floatView = new Float32Array(buffer);
    
    intView[0] = value;
    return floatView[0];
  }

  /**
   * Read flowmeter data from Sealand flowmeter
   * Based on the Lua implementation in tt_flowmeter_sealand service
   */
  async readFlowmeterData(deviceAddress: number): Promise<FlowmeterData | null> {
    if (!this.isConnected) {
      throw new Error('Not connected to RS485. Call connect() first.');
    }

    try {
      this.client.setID(deviceAddress);
      
      // Read 22 registers starting from address 244 (245 - 1)
      const startAddress = 245 - this.flowmeterRegOffset;
      const registerCount = 22;
      
      console.log(`📊 Reading flowmeter data from device ${deviceAddress}, address ${startAddress}, count ${registerCount}`);
      
      const result = await this.client.readHoldingRegisters(startAddress, registerCount);
      const registers = result.data;

      if (registers.length !== registerCount) {
        throw new Error(`Expected ${registerCount} registers, got ${registers.length}`);
      }

      // Parse data according to Sealand flowmeter register mapping
      const flowmeterData: FlowmeterData = {
        deviceAddress,
        timestamp: new Date(),
        // Error code: registers 0-3 (32-bit)
        errorCode: this.registers32ToInt(registers, 2),
        // Mass flow rate: registers 4-7 (32-bit float)
        massFlowRate: this.int32ToFloat(this.registers32ToInt(registers, 4)),
        // Density flow: registers 8-11 (32-bit float)
        densityFlow: this.int32ToFloat(this.registers32ToInt(registers, 8)),
        // Temperature: registers 12-15 (32-bit float)
        temperature: this.int32ToFloat(this.registers32ToInt(registers, 12)),
        // Volume flow rate: registers 16-19 (32-bit float)
        volumeFlowRate: this.int32ToFloat(this.registers32ToInt(registers, 16)),
        // Mass total: registers 28-31 would be at index 20-23, but we only read 22 registers
        // Adjusting based on the 22-register window
        massTotal: registers.length > 20 ? this.int32ToFloat(this.registers32ToInt(registers, 20)) : 0,
        // Volume total: would be next 4 registers
        volumeTotal: 0, // Not available in 22-register window
        // Mass inventory: would be next 4 registers  
        massInventory: 0, // Not available in 22-register window
        // Volume inventory: would be next 4 registers
        volumeInventory: 0 // Not available in 22-register window
      };

      return flowmeterData;

    } catch (error) {
      console.error(`❌ Failed to read flowmeter data from device ${deviceAddress}:`, error);
      return null;
    }
  }

  /**
   * Read complete flowmeter data (all registers)
   * Reads additional registers to get complete data set
   */
  async readCompleteFlowmeterData(deviceAddress: number): Promise<FlowmeterData | null> {
    if (!this.isConnected) {
      throw new Error('Not connected to RS485. Call connect() first.');
    }

    try {
      this.client.setID(deviceAddress);
      
      // Read extended register range to get all data
      const startAddress = 245 - this.flowmeterRegOffset;
      const registerCount = 44; // Extended to include all data
      
      console.log(`📊 Reading complete flowmeter data from device ${deviceAddress}`);
      
      const result = await this.client.readHoldingRegisters(startAddress, registerCount);
      const registers = result.data;

      // Parse complete data set
      const flowmeterData: FlowmeterData = {
        deviceAddress,
        timestamp: new Date(),
        errorCode: this.registers32ToInt(registers, 0),
        massFlowRate: this.int32ToFloat(this.registers32ToInt(registers, 4)),
        densityFlow: this.int32ToFloat(this.registers32ToInt(registers, 8)),
        temperature: this.int32ToFloat(this.registers32ToInt(registers, 12)),
        volumeFlowRate: this.int32ToFloat(this.registers32ToInt(registers, 16)),
        massTotal: this.int32ToFloat(this.registers32ToInt(registers, 28)),
        volumeTotal: this.int32ToFloat(this.registers32ToInt(registers, 32)),
        massInventory: this.int32ToFloat(this.registers32ToInt(registers, 36)),
        volumeInventory: this.int32ToFloat(this.registers32ToInt(registers, 40))
      };

      return flowmeterData;

    } catch (error) {
      console.error(`❌ Failed to read complete flowmeter data from device ${deviceAddress}:`, error);
      return null;
    }
  }

  /**
   * Reset flowmeter accumulation
   * Mimics the resetAccumulation command from the Lua service
   */
  async resetFlowmeterAccumulation(deviceAddress: number): Promise<boolean> {
    if (!this.isConnected) {
      throw new Error('Not connected to RS485. Call connect() first.');
    }

    try {
      this.client.setID(deviceAddress);
      
      // Write to coil 3 (adjusted for offset) to reset accumulation
      const coilAddress = 3 - this.flowmeterRegOffset;
      
      console.log(`🔄 Resetting accumulation for flowmeter device ${deviceAddress}`);
      
      await this.client.writeCoil(coilAddress, true);
      
      console.log(`✅ Successfully reset accumulation for device ${deviceAddress}`);
      return true;

    } catch (error) {
      console.error(`❌ Failed to reset accumulation for device ${deviceAddress}:`, error);
      return false;
    }
  }

  /**
   * Monitor multiple flowmeters continuously
   */
  async monitorFlowmeters(deviceAddresses: number[], intervalMs: number = 5000): Promise<void> {
    console.log(`🔄 Starting flowmeter monitoring for devices: [${deviceAddresses.join(', ')}]`);
    console.log(`📡 Update interval: ${intervalMs}ms`);
    console.log('Press Ctrl+C to stop\n');

    const monitor = setInterval(async () => {
      console.log(`⏰ ${new Date().toISOString()} - Reading flowmeter data...\n`);
      
      for (const deviceAddress of deviceAddresses) {
        const data = await this.readFlowmeterData(deviceAddress);
        
        if (data) {
          console.log(`📊 Device ${deviceAddress} - Sealand Flowmeter:`);
          console.log(`   Error Code: ${data.errorCode}`);
          console.log(`   Mass Flow Rate: ${data.massFlowRate.toFixed(2)} kg/h`);
          console.log(`   Density Flow: ${data.densityFlow.toFixed(4)} kg/m³`);
          console.log(`   Temperature: ${data.temperature.toFixed(2)} °C`);
          console.log(`   Volume Flow Rate: ${data.volumeFlowRate.toFixed(3)} m³/h`);
          console.log(`   Mass Total: ${data.massTotal.toFixed(2)} kg`);
          console.log(`   Volume Total: ${data.volumeTotal.toFixed(3)} m³`);
          console.log('');
        } else {
          console.log(`❌ Failed to read data from device ${deviceAddress}\n`);
        }
      }
    }, intervalMs);

    // Handle graceful shutdown
    process.on('SIGINT', () => {
      console.log('\n🛑 Stopping flowmeter monitor...');
      clearInterval(monitor);
      this.disconnect();
      process.exit(0);
    });
  }

  /**
   * Add a Modbus device configuration
   */
  addDevice(device: ModbusDevice): void {
    this.devices.push(device);
    console.log(`📋 Added device: ${device.name} (ID: ${device.id})`);
  }

  /**
   * Read holding registers from a Modbus device
   */
  async readHoldingRegisters(deviceId: number, address: number, count: number): Promise<number[]> {
    if (!this.isConnected) {
      throw new Error('Not connected to RS485. Call connect() first.');
    }

    try {
      this.client.setID(deviceId);
      const result = await this.client.readHoldingRegisters(address, count);
      return result.data;
    } catch (error) {
      console.error(`❌ Failed to read holding registers from device ${deviceId}:`, error);
      throw error;
    }
  }

  /**
   * Read input registers from a Modbus device
   */
  async readInputRegisters(deviceId: number, address: number, count: number): Promise<number[]> {
    if (!this.isConnected) {
      throw new Error('Not connected to RS485. Call connect() first.');
    }

    try {
      this.client.setID(deviceId);
      const result = await this.client.readInputRegisters(address, count);
      return result.data;
    } catch (error) {
      console.error(`❌ Failed to read input registers from device ${deviceId}:`, error);
      throw error;
    }
  }

  /**
   * Read coils from a Modbus device
   */
  async readCoils(deviceId: number, address: number, count: number): Promise<boolean[]> {
    if (!this.isConnected) {
      throw new Error('Not connected to RS485. Call connect() first.');
    }

    try {
      this.client.setID(deviceId);
      const result = await this.client.readCoils(address, count);
      return result.data;
    } catch (error) {
      console.error(`❌ Failed to read coils from device ${deviceId}:`, error);
      throw error;
    }
  }

  /**
   * Write single holding register
   */
  async writeSingleRegister(deviceId: number, address: number, value: number): Promise<void> {
    if (!this.isConnected) {
      throw new Error('Not connected to RS485. Call connect() first.');
    }

    try {
      this.client.setID(deviceId);
      await this.client.writeRegister(address, value);
      console.log(`✅ Written value ${value} to register ${address} on device ${deviceId}`);
    } catch (error) {
      console.error(`❌ Failed to write register on device ${deviceId}:`, error);
      throw error;
    }
  }

  /**
   * List available serial ports (useful for debugging)
   */
  static async listSerialPorts(): Promise<void> {
    try {
      const ports = await SerialPort.list();
      console.log('Available serial ports:');
      ports.forEach(port => {
        console.log(`  ${port.path} - ${port.manufacturer || 'Unknown'}`);
      });
    } catch (error) {
      console.error('Failed to list serial ports:', error);
    }
  }

  /**
   * Disconnect from RS485
   */
  disconnect(): void {
    if (this.isConnected) {
      this.client.close(() => {
        console.log('🔌 Disconnected from RS485');
      });
      this.isConnected = false;
    }
  }
}

// Example usage for flowmeter monitoring
async function main() {
  const reader = new BL410ModbusReader();

  try {
    console.log('🖥️  BL410 Sealand Flowmeter Reader\n');
    await BL410ModbusReader.listSerialPorts();
    console.log('');

    // Connect to RS485
    await reader.connect('/dev/ttyS0', 9600, 'none');

    // Example: Read data from flowmeter devices (adjust addresses as needed)
    const flowmeterAddresses = [1, 2, 3, 4]; // Device addresses from your system

    // Single read example
    console.log('📊 Single read example:\n');
    for (const address of flowmeterAddresses) {
      const data = await reader.readFlowmeterData(address);
      if (data) {
        console.log(`Flowmeter ${address}:`, {
          errorCode: data.errorCode,
          massFlowRate: data.massFlowRate,
          temperature: data.temperature,
          volumeTotal: data.volumeTotal
        });
      }
    }

    // Ask user for continuous monitoring
    const readline = require('readline').createInterface({
      input: process.stdin,
      output: process.stdout
    });

    readline.question('\nStart continuous flowmeter monitoring? (y/n): ', (answer:string) => {
      readline.close();
      if (answer.toLowerCase() === 'y' || answer.toLowerCase() === 'yes') {
        reader.monitorFlowmeters(flowmeterAddresses, 10000); // Every 10 seconds
      } else {
        reader.disconnect();
        console.log('👋 Goodbye!');
        process.exit(0);
      }
    });

  } catch (error) {
    console.error('Main error:', error);
    reader.disconnect();
    process.exit(1);
  }
}

// Export for use as module
export { BL410ModbusReader, ModbusDevice, FlowmeterData };

// Run if this file is executed directly
if (require.main === module) {
  main();
}
