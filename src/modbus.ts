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
  async connect(port: string = '/dev/ttyS0', baudRate: number = 9600, parity: 'even'): Promise<void> {
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
   * For 2-register values (32-bit) - FIXED VERSION
   */
  private registers32ToInt(registers: number[], startIndex: number): number {
    // Use only 2 registers for 32-bit value
    const high16 = registers[startIndex];
    const low16 = registers[startIndex + 1];
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

      console.log(`Raw registers: [${registers.join(', ')}]`); // Debug log

      // Parse data according to Sealand flowmeter register mapping
      // Each 32-bit value uses 2 consecutive 16-bit registers
      const flowmeterData: FlowmeterData = {
        deviceAddress,
        timestamp: new Date(),
        // Error code: registers 0-1 (32-bit, 2 registers)
        errorCode: this.registers32ToInt(registers, 0),
        // Mass flow rate: registers 2-3 (32-bit float, 2 registers)
        massFlowRate: this.int32ToFloat(this.registers32ToInt(registers, 2)),
        // Density flow: registers 4-5 (32-bit float, 2 registers)
        densityFlow: this.int32ToFloat(this.registers32ToInt(registers, 4)),
        // Temperature: registers 6-7 (32-bit float, 2 registers)
        temperature: this.int32ToFloat(this.registers32ToInt(registers, 6)),
        // Volume flow rate: registers 8-9 (32-bit float, 2 registers)
        volumeFlowRate: this.int32ToFloat(this.registers32ToInt(registers, 8)),
        // Mass total: registers 10-11 (32-bit float, 2 registers)
        massTotal: this.int32ToFloat(this.registers32ToInt(registers, 10)),
        // Volume total: registers 12-13 (32-bit float, 2 registers)
        volumeTotal: this.int32ToFloat(this.registers32ToInt(registers, 12)),
        // Mass inventory: registers 14-15 (32-bit float, 2 registers)
        massInventory: this.int32ToFloat(this.registers32ToInt(registers, 14)),
        // Volume inventory: registers 16-17 (32-bit float, 2 registers)
        volumeInventory: this.int32ToFloat(this.registers32ToInt(registers, 16))
      };

      console.log(`Parsed data:`, {
        errorCode: flowmeterData.errorCode,
        massFlowRate: flowmeterData.massFlowRate,
        temperature: flowmeterData.temperature,
        volumeFlowRate: flowmeterData.volumeFlowRate
      }); // Debug log

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
      const registerCount = 22; // Keep same as basic read for consistency
      
      console.log(`📊 Reading complete flowmeter data from device ${deviceAddress}`);
      
      const result = await this.client.readHoldingRegisters(startAddress, registerCount);
      const registers = result.data;

      console.log(`Complete raw registers: [${registers.join(', ')}]`); // Debug log

      // Parse complete data set with correct 2-register mapping
      const flowmeterData: FlowmeterData = {
        deviceAddress,
        timestamp: new Date(),
        errorCode: this.registers32ToInt(registers, 0),
        massFlowRate: this.int32ToFloat(this.registers32ToInt(registers, 2)),
        densityFlow: this.int32ToFloat(this.registers32ToInt(registers, 4)),
        temperature: this.int32ToFloat(this.registers32ToInt(registers, 6)),
        volumeFlowRate: this.int32ToFloat(this.registers32ToInt(registers, 8)),
        massTotal: this.int32ToFloat(this.registers32ToInt(registers, 10)),
        volumeTotal: this.int32ToFloat(this.registers32ToInt(registers, 12)),
        massInventory: this.int32ToFloat(this.registers32ToInt(registers, 14)),
        volumeInventory: this.int32ToFloat(this.registers32ToInt(registers, 16))
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
    await reader.connect('/dev/ttyS0', 9600, 'even');

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
