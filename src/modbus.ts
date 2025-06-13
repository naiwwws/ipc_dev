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
  private serialPort: SerialPort | null = null;
  private isConnected: boolean = false;
  private devices: ModbusDevice[] = [];
  private flowmeterRegOffset: number = 1;
  private responseBuffer: Buffer = Buffer.alloc(0);

  constructor() {
    // Using raw SerialPort with custom CRC functions
  }

  /**
   * Initialize RS485 connection with raw serial port
   * @param port Serial port (e.g., '/dev/ttyS0' for BL410)
   * @param baudRate Baud rate (default: 9600)
   * @param parity Parity setting (default: 'even')
   */
  async connect(port: string = '/dev/ttyS0', baudRate: number = 9600, parity: 'even' | 'none' | 'odd' = 'even'): Promise<void> {
    try {
      console.log(`Connecting to RS485 port: ${port}`);
      console.log(`Baud rate: ${baudRate}, Parity: ${parity}`);

      this.serialPort = new SerialPort({
        path: port,
        baudRate: baudRate,
        dataBits: 8,
        stopBits: 1,
        parity: parity,
        autoOpen: false
      });

      return new Promise((resolve, reject) => {
        this.serialPort!.open((err) => {
          if (err) {
            reject(err);
            return;
          }

          // Set up data handler for raw bytes
          this.serialPort!.on('data', (data: Buffer) => {
            this.responseBuffer = Buffer.concat([this.responseBuffer, data]);
          });

          this.isConnected = true;
          console.log('✅ RS485 connection established');
          resolve();
        });
      });

    } catch (error) {
      console.error('❌ Failed to connect to RS485:', error);
      throw error;
    }
  }

  /**
   * Disconnect from RS485
   */
  disconnect(): void {
    if (this.isConnected && this.serialPort) {
      this.serialPort.close(() => {
        console.log('✅ RS485 connection closed');
      });
      this.isConnected = false;
      this.serialPort = null;
    }
  }

  /**
   * List available serial ports
   */
  static async listSerialPorts(): Promise<void> {
    try {
      const ports = await SerialPort.list();
      console.log('📡 Available Serial Ports:');
      
      if (ports.length === 0) {
        console.log('   No serial ports found');
        return;
      }

      ports.forEach((port, index) => {
        console.log(`   ${index + 1}. ${port.path}`);
        if (port.manufacturer) {
          console.log(`      Manufacturer: ${port.manufacturer}`);
        }
        if (port.serialNumber) {
          console.log(`      Serial Number: ${port.serialNumber}`);
        }
        if (port.pnpId) {
          console.log(`      PnP ID: ${port.pnpId}`);
        }
        console.log('');
      });
    } catch (error) {
      console.error('❌ Failed to list serial ports:', error);
    }
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
   * Calculate CRC-16 Modbus checksum
   * Standard Modbus CRC-16 implementation
   * @param data Buffer containing data to calculate CRC for
   * @returns CRC-16 value
   */
  private calculateCRC16(data: Buffer): number {
    let crc = 0xFFFF;
    
    for (let i = 0; i < data.length; i++) {
      crc ^= data[i];
      
      for (let j = 0; j < 8; j++) {
        if (crc & 0x0001) {
          crc = (crc >> 1) ^ 0xA001;
        } else {
          crc = crc >> 1;
        }
      }
    }
    
    return crc;
  }

  /**
   * Create Modbus Read Holding Registers frame with CRC
   * @param address Device address
   * @param startRegisterAddr Starting register address
   * @param quantity Number of registers to read
   * @returns Complete Modbus frame with CRC
   */
  private createReadHoldingRegistersFrame(address: number, startRegisterAddr: number, quantity: number): Buffer {
    const frame = Buffer.alloc(6);
    frame[0] = address;
    frame[1] = 0x03; // Function code for Read Holding Registers
    frame[2] = (startRegisterAddr >>> 8) & 0xFF; // Start address high byte
    frame[3] = startRegisterAddr & 0xFF;          // Start address low byte
    frame[4] = (quantity >>> 8) & 0xFF;           // Quantity high byte
    frame[5] = quantity & 0xFF;                    // Quantity low byte
    
    // Calculate CRC and append to frame
    const crc = this.calculateCRC16(frame);
    const frameWithCrc = Buffer.alloc(8);
    frame.copy(frameWithCrc, 0);
    frameWithCrc[6] = crc & 0xFF;        // CRC low byte
    frameWithCrc[7] = (crc >>> 8) & 0xFF; // CRC high byte
    
    return frameWithCrc;
  }

  /**
   * Verify Modbus frame CRC
   * @param frameData Complete frame data including CRC bytes
   * @returns true if CRC is valid
   */
  private verifyModbusCRC(frameData: Buffer): boolean {
    if (frameData.length < 4) {
      return false;
    }
    
    // Extract data without CRC (all bytes except last 2)
    const dataWithoutCrc = frameData.slice(0, frameData.length - 2);
    
    // Calculate CRC for data
    const calculatedCrc = this.calculateCRC16(dataWithoutCrc);
    
    // Extract received CRC (last 2 bytes)
    const receivedCrcLow = frameData[frameData.length - 2];
    const receivedCrcHigh = frameData[frameData.length - 1];
    const receivedCrc = (receivedCrcHigh << 8) | receivedCrcLow;
    
    // Compare calculated vs received CRC
    const isValid = calculatedCrc === receivedCrc;
    
    if (!isValid) {
      console.log(`🔍 CRC Mismatch: calculated=0x${calculatedCrc.toString(16)}, received=0x${receivedCrc.toString(16)}`);
    }
    
    return isValid;
  }

  /**
   * Send raw Modbus frame and wait for response
   */
  private async sendRawModbusFrame(frame: Buffer, expectedResponseLength: number, timeoutMs: number = 3000): Promise<Buffer | null> {
    if (!this.isConnected || !this.serialPort) {
      throw new Error('Not connected to RS485. Call connect() first.');
    }

    return new Promise((resolve) => {
      // Clear response buffer
      this.responseBuffer = Buffer.alloc(0);

      // Send the frame
      console.log(`📤 Sending raw frame: [${Array.from(frame).join(', ')}]`);
      
      this.serialPort!.write(frame, (writeErr) => {
        if (writeErr) {
          console.error('❌ Write error:', writeErr);
          resolve(null);
          return;
        }
      });

      // Set timeout for response
      const timeout = setTimeout(() => {
        console.log('⏰ Response timeout');
        resolve(null);
      }, timeoutMs);

      // Check for response periodically
      const checkResponse = setInterval(() => {
        if (this.responseBuffer.length >= expectedResponseLength) {
          clearTimeout(timeout);
          clearInterval(checkResponse);

          // Get the exact response length
          const response = this.responseBuffer.slice(0, expectedResponseLength);
          console.log(`📥 Received response: [${Array.from(response).join(', ')}]`);

          // Verify CRC
          if (this.verifyModbusCRC(response)) {
            console.log('✅ CRC validation passed');
            resolve(response);
          } else {
            console.log('❌ CRC validation failed');
            resolve(null);
          }
        }
      }, 10); // Check every 10ms
    });
  }

  /**
   * Read flowmeter data using CRC validation
   */
  async readFlowmeterData(deviceAddress: number): Promise<FlowmeterData | null> {
    if (!this.isConnected) {
      throw new Error('Not connected to RS485. Call connect() first.');
    }

    try {
      // Read 22 registers starting from address 244 (245 - 1) - matching Lua exactly
      const startAddress = 245 - this.flowmeterRegOffset;
      const registerCount = 22;
      
      console.log(`📊 Reading flowmeter data from device ${deviceAddress}, address ${startAddress}, count ${registerCount}`);
      
      // Create Modbus frame with CRC
      const frame = this.createReadHoldingRegistersFrame(deviceAddress, startAddress, registerCount);
      
      // Expected response length: Device ID (1) + Function Code (1) + Byte Count (1) + Data (44 bytes) + CRC (2) = 49 bytes
      const expectedResponseLength = 1 + 1 + 1 + (registerCount * 2) + 2;
      
      // Send frame and wait for response
      const response = await this.sendRawModbusFrame(frame, expectedResponseLength, 5000);
      
      if (!response) {
        console.log(`❌ No valid response from device ${deviceAddress}`);
        return null;
      }

      // Extract data from response (skip device ID, function code, byte count, and CRC)
      // Response format: [Device ID][Function Code][Byte Count][Data...][CRC Low][CRC High]
      const dataBytes = Array.from(response.slice(3, response.length - 2)); // Remove first 3 bytes and last 2 CRC bytes
      
      if (dataBytes.length !== registerCount * 2) {
        console.log(`❌ Unexpected data length: expected ${registerCount * 2}, got ${dataBytes.length}`);
        return null;
      }

      console.log(`✅ Cleaned data bytes: [${dataBytes.join(', ')}]`);

      // Parse data according to Lua flowmeter register mapping
      const flowmeterData: FlowmeterData = {
        deviceAddress,
        timestamp: new Date(),
        // Error code: bytes 0-3 (32-bit) - matching Lua bit operations
        errorCode: ((dataBytes[0] << 8) | dataBytes[1]) << 16 | ((dataBytes[2] << 8) | dataBytes[3]),
        // Mass flow rate: bytes 4-7 (32-bit float)
        massFlowRate: this.int32ToFloat(((dataBytes[4] << 8) | dataBytes[5]) << 16 | ((dataBytes[6] << 8) | dataBytes[7])),
        // Density flow: bytes 8-11 (32-bit float)
        densityFlow: this.int32ToFloat(((dataBytes[8] << 8) | dataBytes[9]) << 16 | ((dataBytes[10] << 8) | dataBytes[11])),
        // Temperature: bytes 12-15 (32-bit float)
        temperature: this.int32ToFloat(((dataBytes[12] << 8) | dataBytes[13]) << 16 | ((dataBytes[14] << 8) | dataBytes[15])),
        // Volume flow rate: bytes 16-19 (32-bit float)
        volumeFlowRate: this.int32ToFloat(((dataBytes[16] << 8) | dataBytes[17]) << 16 | ((dataBytes[18] << 8) | dataBytes[19])),
        // Mass total: bytes 28-31 (32-bit float) - note Lua uses arg[29]-arg[32] (0-based vs 1-based)
        massTotal: this.int32ToFloat(((dataBytes[28] << 8) | dataBytes[29]) << 16 | ((dataBytes[30] << 8) | dataBytes[31])),
        // Volume total: bytes 32-35 (32-bit float)
        volumeTotal: this.int32ToFloat(((dataBytes[32] << 8) | dataBytes[33]) << 16 | ((dataBytes[34] << 8) | dataBytes[35])),
        // Mass inventory: bytes 36-39 (32-bit float)
        massInventory: this.int32ToFloat(((dataBytes[36] << 8) | dataBytes[37]) << 16 | ((dataBytes[38] << 8) | dataBytes[39])),
        // Volume inventory: bytes 40-43 (32-bit float)
        volumeInventory: this.int32ToFloat(((dataBytes[40] << 8) | dataBytes[41]) << 16 | ((dataBytes[42] << 8) | dataBytes[43]))
      };

      console.log(`✅ Successfully parsed data from device ${deviceAddress}:`, {
        errorCode: flowmeterData.errorCode,
        massFlowRate: flowmeterData.massFlowRate,
        temperature: flowmeterData.temperature,
        volumeFlowRate: flowmeterData.volumeFlowRate
      });

      return flowmeterData;

    } catch (error) {
      console.error(`❌ Failed to read flowmeter data from device ${deviceAddress}:`, error);
      return null;
    }
  }

  /**
   * Test if a device is responsive using raw communication
   * @param deviceAddress Device address to test
   * @returns true if device responds, false otherwise
   */
  async testDeviceResponsive(deviceAddress: number): Promise<boolean> {
    if (!this.isConnected) {
      return false;
    }

    try {
      // Try to read just 1 register to test if device is responsive
      const frame = this.createReadHoldingRegistersFrame(deviceAddress, 244, 1);
      const expectedResponseLength = 1 + 1 + 1 + 2 + 2; // Device ID + Function Code + Byte Count + 2 data bytes + CRC
      
      const response = await this.sendRawModbusFrame(frame, expectedResponseLength, 2000);
      
      if (response) {
        console.log(`✅ Device ${deviceAddress} is responsive`);
        return true;
      } else {
        console.log(`📵 Device ${deviceAddress} is not responsive or not connected`);
        return false;
      }
    } catch (error) {
      console.log(`📵 Device ${deviceAddress} test failed:`, error);
      return false;
    }
  }

  /**
   * Scan for available devices on the bus
   * @param addressRange Array of addresses to scan
   * @returns Array of responsive device addresses
   */
  async scanForDevices(addressRange: number[] = [1, 2, 3, 4, 5, 6, 7, 8]): Promise<number[]> {
    console.log(`🔍 Scanning for devices on addresses: [${addressRange.join(', ')}]`);
    const responsiveDevices: number[] = [];

    for (const address of addressRange) {
      console.log(`🔍 Testing device ${address}...`);
      const isResponsive = await this.testDeviceResponsive(address);
      
      if (isResponsive) {
        responsiveDevices.push(address);
      }
      
      // Add delay between device tests
      await new Promise(resolve => setTimeout(resolve, 500));
    }

    console.log(`✅ Found responsive devices: [${responsiveDevices.join(', ')}]`);
    return responsiveDevices;
  }

  /**
   * Monitor multiple flowmeters continuously
   */
  async monitorFlowmeters(deviceAddresses: number[], intervalMs: number = 5000): Promise<void> {
    console.log(`🔄 Starting flowmeter monitoring for devices: [${deviceAddresses.join(', ')}]`);
    console.log(`📡 Update interval: ${intervalMs}ms`);
    console.log('Press Ctrl+C to stop\n');

    // First, scan for responsive devices
    const responsiveDevices = await this.scanForDevices(deviceAddresses);
    
    if (responsiveDevices.length === 0) {
      console.log('❌ No responsive devices found. Check connections and device addresses.');
      return;
    }

    const monitor = setInterval(async () => {
      console.log(`⏰ ${new Date().toISOString()} - Reading flowmeter data...\n`);
      
      // Only read from responsive devices
      for (const deviceAddress of responsiveDevices) {
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
          console.log(`📵 Device ${deviceAddress} - No data (device may be offline)\n`);
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

    // Example: Device addresses to scan
    const flowmeterAddresses = [1, 2, 3, 4, 5, 6, 7, 8];

    // Scan for responsive devices first
    console.log('🔍 Scanning for responsive devices...\n');
    const responsiveDevices = await reader.scanForDevices(flowmeterAddresses);
    
    if (responsiveDevices.length === 0) {
      console.log('❌ No responsive devices found. Check your connections and device addresses.');
      reader.disconnect();
      return;
    }

    // Single read example from responsive devices only
    console.log('📊 Single read example from responsive devices:\n');
    for (const address of responsiveDevices) {
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
        reader.monitorFlowmeters(responsiveDevices, 10000); // Every 10 seconds
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
