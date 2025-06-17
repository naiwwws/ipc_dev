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
  private debugMode: boolean = true;

  constructor() {
    // Using raw SerialPort with consistent CRC handling
  }

  /**
   * Initialize RS485 connection with enhanced error handling
   */
  async connect(port: string = '/dev/ttyS0', baudRate: number = 9600, parity: 'even' | 'none' | 'odd' = 'even'): Promise<void> {
    try {
      console.log(`🔌 Connecting to RS485 port: ${port}`);
      console.log(`⚙️  Configuration: ${baudRate} baud, ${parity} parity, 8 data bits, 1 stop bit`);

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
            console.error('❌ Failed to open serial port:', err);
            reject(err);
            return;
          }

          // Set up data handler for raw bytes
          this.serialPort!.on('data', (data: Buffer) => {
            this.responseBuffer = Buffer.concat([this.responseBuffer, data]);
            if (this.debugMode) {
              console.log(`📥 Raw data received: [${Array.from(data).map(b => `0x${b.toString(16).padStart(2, '0')}`).join(', ')}]`);
            }
          });

          // Enhanced error handling
          this.serialPort!.on('error', (error) => {
            console.error('🚨 Serial port error:', error);
          });

          this.serialPort!.on('close', () => {
            console.log('🔐 Serial port closed');
            this.isConnected = false;
          });

          this.isConnected = true;
          console.log('✅ RS485 connection established successfully');
          resolve();
        });
      });

    } catch (error) {
      console.error('❌ Failed to connect to RS485:', error);
      throw error;
    }
  }

  /**
   * Disconnect from RS485 with cleanup
   */
  disconnect(): void {
    if (this.isConnected && this.serialPort) {
      this.serialPort.close(() => {
        console.log('✅ RS485 connection closed gracefully');
      });
      this.isConnected = false;
      this.serialPort = null;
      this.responseBuffer = Buffer.alloc(0);
    }
  }

  /**
   * Enhanced serial port listing
   */
  static async listSerialPorts(): Promise<void> {
    try {
      const ports = await SerialPort.list();
      console.log('📡 Available Serial Ports:');
      
      if (ports.length === 0) {
        console.log('   ⚠️  No serial ports found');
        return;
      }

      ports.forEach((port, index) => {
        console.log(`   ${index + 1}. ${port.path}`);
        if (port.manufacturer) console.log(`      📱 Manufacturer: ${port.manufacturer}`);
        if (port.serialNumber) console.log(`      🔢 Serial Number: ${port.serialNumber}`);
        if (port.pnpId) console.log(`      🏷️  PnP ID: ${port.pnpId}`);
        console.log('');
      });
    } catch (error) {
      console.error('❌ Failed to list serial ports:', error);
    }
  }

  /**
   * Convert 32-bit integer to IEEE 754 float (Lua compatible)
   */
  private int32ToFloat(value: number): number {
    const buffer = new ArrayBuffer(4);
    const intView = new Int32Array(buffer);
    const floatView = new Float32Array(buffer);
    
    intView[0] = value;
    return floatView[0];
  }

  /**
   * Enhanced CRC-16 Modbus calculation with validation
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
    
    return crc & 0xFFFF; // Ensure 16-bit result
  }

  /**
   * Create Modbus Read Holding Registers frame with CONSISTENT CRC byte order
   */
  private createReadHoldingRegistersFrame(address: number, startRegisterAddr: number, quantity: number): Buffer {
    // Validate inputs
    if (address < 1 || address > 247) {
      throw new Error(`Invalid device address: ${address}. Must be 1-247`);
    }
    if (startRegisterAddr < 0 || startRegisterAddr > 65535) {
      throw new Error(`Invalid register address: ${startRegisterAddr}. Must be 0-65535`);
    }
    if (quantity < 1 || quantity > 125) {
      throw new Error(`Invalid quantity: ${quantity}. Must be 1-125`);
    }

    const frame = Buffer.alloc(6);
    frame[0] = address;
    frame[1] = 0x03; // Function code for Read Holding Registers  
    frame[2] = (startRegisterAddr >>> 8) & 0xFF; // Start address high byte
    frame[3] = startRegisterAddr & 0xFF;         // Start address low byte
    frame[4] = (quantity >>> 8) & 0xFF;          // Quantity high byte
    frame[5] = quantity & 0xFF;                  // Quantity low byte
    
    // Calculate CRC and append with CONSISTENT byte order (little-endian)
    const crc = this.calculateCRC16(frame);
    const frameWithCrc = Buffer.alloc(8);
    frame.copy(frameWithCrc, 0);
    
    // MODBUS CRC is transmitted LITTLE-ENDIAN (low byte first, then high byte)
    frameWithCrc[6] = crc & 0xFF;         // CRC low byte first
    frameWithCrc[7] = (crc >>> 8) & 0xFF; // CRC high byte second
    
    if (this.debugMode) {
      console.log(`🔧 Created frame: [${Array.from(frameWithCrc).map(b => `0x${b.toString(16).padStart(2, '0')}`).join(', ')}]`);
      console.log(`🔧 CRC calculated: 0x${crc.toString(16).padStart(4, '0')} (Low: 0x${(crc & 0xFF).toString(16).padStart(2, '0')}, High: 0x${((crc >>> 8) & 0xFF).toString(16).padStart(2, '0')})`);
    }
    
    return frameWithCrc;
  }

  /**
   * Enhanced CRC verification with CONSISTENT byte order handling
   */
  private verifyModbusCRC(frameData: Buffer): boolean {
    if (frameData.length < 4) {
      console.log('❌ Frame too short for CRC validation');
      return false;
    }
    
    // Extract data without CRC (all bytes except last 2)
    const dataWithoutCrc = frameData.slice(0, frameData.length - 2);
    
    // Calculate CRC for data
    const calculatedCrc = this.calculateCRC16(dataWithoutCrc);
    
    // Extract received CRC bytes (last 2 bytes)
    const receivedCrcLow = frameData[frameData.length - 2];  // Low byte
    const receivedCrcHigh = frameData[frameData.length - 1]; // High byte
    
    // MODBUS CRC is transmitted LITTLE-ENDIAN: reconstruct consistently
    const receivedCrc = (receivedCrcHigh << 8) | receivedCrcLow;
    
    if (this.debugMode) {
      console.log(`🔍 CRC Verification:`);
      console.log(`   📊 Frame: [${Array.from(frameData).map(b => `0x${b.toString(16).padStart(2, '0')}`).join(', ')}]`);
      console.log(`   📋 Data: [${Array.from(dataWithoutCrc).map(b => `0x${b.toString(16).padStart(2, '0')}`).join(', ')}]`);
      console.log(`   📥 Received CRC bytes: Low=0x${receivedCrcLow.toString(16).padStart(2, '0')}, High=0x${receivedCrcHigh.toString(16).padStart(2, '0')}`);
      console.log(`   📐 Calculated CRC: 0x${calculatedCrc.toString(16).padStart(4, '0')}`);
      console.log(`   📨 Received CRC: 0x${receivedCrc.toString(16).padStart(4, '0')}`);
    }
    
    // Primary check with consistent byte order
    if (calculatedCrc === receivedCrc) {
      if (this.debugMode) console.log('✅ CRC validation passed (standard byte order)');
      return true;
    }
    
    // Fallback: try reverse byte order for compatibility
    const receivedCrcReversed = (receivedCrcLow << 8) | receivedCrcHigh;
    if (calculatedCrc === receivedCrcReversed) {
      if (this.debugMode) console.log('✅ CRC validation passed (reversed byte order)');
      return true;
    }
    
    console.log(`❌ CRC validation failed: calculated=0x${calculatedCrc.toString(16).padStart(4, '0')}, received=0x${receivedCrc.toString(16).padStart(4, '0')}, reversed=0x${receivedCrcReversed.toString(16).padStart(4, '0')}`);
    return false;
  }

  /**
   * Enhanced raw Modbus frame sender with dynamic response detection
   */
  private async sendRawModbusFrame(frame: Buffer, timeoutMs: number = 3000): Promise<Buffer | null> {
    if (!this.isConnected || !this.serialPort) {
      throw new Error('Not connected to RS485. Call connect() first.');
    }

    return new Promise((resolve) => {
      // Clear response buffer and flags
      this.responseBuffer = Buffer.alloc(0);
      let responseComplete = false;
      let timeout: NodeJS.Timeout;
      let checkInterval: NodeJS.Timeout;

      // Send the frame
      console.log(`📤 Sending frame: [${Array.from(frame).map(b => `0x${b.toString(16).padStart(2, '0')}`).join(', ')}]`);
      
      this.serialPort!.write(frame, (writeErr) => {
        if (writeErr) {
          console.error('❌ Write error:', writeErr);
          resolve(null);
          return;
        }
      });

      // Set timeout for response
      timeout = setTimeout(() => {
        if (!responseComplete) {
          console.log('⏰ Response timeout');
          clearInterval(checkInterval);
          resolve(null);
        }
      }, timeoutMs);

      // Dynamic response detection with enhanced logic
      checkInterval = setInterval(() => {
        if (responseComplete) return;

        // Need at least 5 bytes for minimum valid response
        if (this.responseBuffer.length >= 5) {
          
          // Check for Modbus exception/error response
          if (this.responseBuffer[1] & 0x80) {
            // Error response format: [Address][Function+0x80][Exception Code][CRC Low][CRC High] = 5 bytes
            if (this.responseBuffer.length >= 5) {
              const errorResponse = this.responseBuffer.slice(0, 5);
              console.log(`📥 Error response: [${Array.from(errorResponse).map(b => `0x${b.toString(16).padStart(2, '0')}`).join(', ')}]`);
              
              clearTimeout(timeout);
              clearInterval(checkInterval);
              responseComplete = true;
              
              if (this.verifyModbusCRC(errorResponse)) {
                console.log('✅ Error response CRC valid');
                resolve(errorResponse);
              } else {
                console.log('❌ Error response CRC invalid');
                resolve(null);
              }
            }
            return;
          }

          // Normal response for function 0x03 (Read Holding Registers)
          if (this.responseBuffer[1] === 0x03 && this.responseBuffer.length >= 3) {
            const byteCount = this.responseBuffer[2];
            
            // Validate byte count
            if (byteCount > 250) { // Reasonable maximum
              console.log(`❌ Invalid byte count: ${byteCount}`);
              clearTimeout(timeout);
              clearInterval(checkInterval);
              resolve(null);
              return;
            }
            
            const expectedTotalLength = 3 + byteCount + 2; // Address + Function + ByteCount + Data + CRC
            
            if (this.debugMode) {
              console.log(`📏 Expected response length: ${expectedTotalLength}, current: ${this.responseBuffer.length}, byte count: ${byteCount}`);
            }
            
            if (this.responseBuffer.length >= expectedTotalLength) {
              const response = this.responseBuffer.slice(0, expectedTotalLength);
              console.log(`📥 Complete response: [${Array.from(response).map(b => `0x${b.toString(16).padStart(2, '0')}`).join(', ')}]`);

              clearTimeout(timeout);
              clearInterval(checkInterval);
              responseComplete = true;

              if (this.verifyModbusCRC(response)) {
                resolve(response);
              } else {
                // Try alternative lengths in case of extra bytes
                console.log('🔄 Trying alternative response lengths...');
                for (let altLen = Math.max(5, expectedTotalLength - 2); altLen <= Math.min(this.responseBuffer.length, expectedTotalLength + 5); altLen++) {
                  const altResponse = this.responseBuffer.slice(0, altLen);
                  if (this.verifyModbusCRC(altResponse)) {
                    console.log(`✅ CRC validation passed with length ${altLen}`);
                    resolve(altResponse);
                    return;
                  }
                }
                console.log('❌ All CRC validation attempts failed');
                resolve(null);
              }
            }
          }
        }
      }, 10); // Check every 10ms
    });
  }

  /**
   * Enhanced device responsiveness test with multiple attempts
   */
  async testDeviceResponsive(deviceAddress: number): Promise<boolean> {
    if (!this.isConnected) {
      return false;
    }

    const maxAttempts = 3;
    for (let attempt = 1; attempt <= maxAttempts; attempt++) {
      try {
        console.log(`🔍 Testing device ${deviceAddress} (attempt ${attempt}/${maxAttempts})...`);
        
        // Try to read 1 register from a commonly available address
        const frame = this.createReadHoldingRegistersFrame(deviceAddress, 244, 1);
        const response = await this.sendRawModbusFrame(frame, 2000);
        
        if (response) {
          // Check if it's an error response
          if (response[1] & 0x80) {
            const errorCode = response[2];
            console.log(`⚠️  Device ${deviceAddress} responded with Modbus error 0x${errorCode.toString(16)} but is reachable`);
            return true; // Device is responsive even if it returns an error
          } else {
            console.log(`✅ Device ${deviceAddress} is responsive and functional`);
            return true;
          }
        }
        
        // Wait between attempts
        if (attempt < maxAttempts) {
          await new Promise(resolve => setTimeout(resolve, 1000));
        }
        
      } catch (error) {
        console.log(`📵 Device ${deviceAddress} test attempt ${attempt} failed:`, error);
      }
    }
    
    console.log(`📵 Device ${deviceAddress} is not responsive after ${maxAttempts} attempts`);
    return false;
  }

  /**
   * Enhanced flowmeter data reading with comprehensive error handling
   */
  async readFlowmeterData(deviceAddress: number): Promise<FlowmeterData | null> {
    if (!this.isConnected) {
      throw new Error('Not connected to RS485. Call connect() first.');
    }

    try {
      // Read 22 registers starting from calculated address
      const startAddress = 245 - this.flowmeterRegOffset; // Should be 244
      const registerCount = 22;
      
      console.log(`📊 Reading flowmeter data from device ${deviceAddress}`);
      console.log(`   📍 Register address: ${startAddress}, count: ${registerCount}`);
      
      // Create and send Modbus frame
      const frame = this.createReadHoldingRegistersFrame(deviceAddress, startAddress, registerCount);
      const response = await this.sendRawModbusFrame(frame, 5000);
      
      if (!response) {
        console.log(`❌ No valid response from device ${deviceAddress}`);
        return null;
      }

      // Check for error response
      if (response[1] & 0x80) {
        const errorCode = response[2];
        console.log(`❌ Device ${deviceAddress} returned Modbus error: 0x${errorCode.toString(16)}`);
        
        // Map common Modbus error codes
        const errorMessages: { [key: number]: string } = {
          0x01: 'Illegal Function',
          0x02: 'Illegal Data Address',
          0x03: 'Illegal Data Value',
          0x04: 'Server Device Failure',
          0x05: 'Acknowledge',
          0x06: 'Server Device Busy'
        };
        
        console.log(`   📝 Error description: ${errorMessages[errorCode] || 'Unknown Error'}`);
        return null;
      }

      // Validate response structure
      if (response.length < 7 || response[1] !== 0x03) {
        console.log(`❌ Invalid response structure from device ${deviceAddress}`);
        return null;
      }

      const byteCount = response[2];
      const expectedDataBytes = registerCount * 2;
      
      if (byteCount !== expectedDataBytes) {
        console.log(`❌ Unexpected byte count: expected ${expectedDataBytes}, got ${byteCount}`);
        return null;
      }

      // Extract data bytes (skip address, function, byte count; remove CRC)
      const dataBytes = Array.from(response.slice(3, 3 + byteCount));
      
      console.log(`✅ Data extracted successfully (${dataBytes.length} bytes)`);
      if (this.debugMode) {
        console.log(`   📊 Data: [${dataBytes.map(b => `0x${b.toString(16).padStart(2, '0')}`).join(', ')}]`);
      }

      // Parse flowmeter data with error handling
      try {
        const flowmeterData: FlowmeterData = {
          deviceAddress,
          timestamp: new Date(),
          // Error code: bytes 0-3 (32-bit big-endian)
          errorCode: ((dataBytes[0] << 8) | dataBytes[1]) << 16 | ((dataBytes[2] << 8) | dataBytes[3]),
          // Mass flow rate: bytes 4-7 (32-bit float)
          massFlowRate: this.int32ToFloat(((dataBytes[4] << 8) | dataBytes[5]) << 16 | ((dataBytes[6] << 8) | dataBytes[7])),
          // Density flow: bytes 8-11 (32-bit float)
          densityFlow: this.int32ToFloat(((dataBytes[8] << 8) | dataBytes[9]) << 16 | ((dataBytes[10] << 8) | dataBytes[11])),
          // Temperature: bytes 12-15 (32-bit float)
          temperature: this.int32ToFloat(((dataBytes[12] << 8) | dataBytes[13]) << 16 | ((dataBytes[14] << 8) | dataBytes[15])),
          // Volume flow rate: bytes 16-19 (32-bit float)
          volumeFlowRate: this.int32ToFloat(((dataBytes[16] << 8) | dataBytes[17]) << 16 | ((dataBytes[18] << 8) | dataBytes[19])),
          // Mass total: bytes 28-31 (32-bit float)
          massTotal: this.int32ToFloat(((dataBytes[28] << 8) | dataBytes[29]) << 16 | ((dataBytes[30] << 8) | dataBytes[31])),
          // Volume total: bytes 32-35 (32-bit float)
          volumeTotal: this.int32ToFloat(((dataBytes[32] << 8) | dataBytes[33]) << 16 | ((dataBytes[34] << 8) | dataBytes[35])),
          // Mass inventory: bytes 36-39 (32-bit float)
          massInventory: this.int32ToFloat(((dataBytes[36] << 8) | dataBytes[37]) << 16 | ((dataBytes[38] << 8) | dataBytes[39])),
          // Volume inventory: bytes 40-43 (32-bit float)
          volumeInventory: this.int32ToFloat(((dataBytes[40] << 8) | dataBytes[41]) << 16 | ((dataBytes[42] << 8) | dataBytes[43]))
        };

        // Validate parsed data ranges
        if (isNaN(flowmeterData.massFlowRate) || isNaN(flowmeterData.temperature)) {
          console.log('⚠️  Warning: Some float values are NaN, data may be corrupted');
        }

        console.log(`✅ Successfully parsed flowmeter data from device ${deviceAddress}:`);
        console.log(`   🏷️  Error Code: 0x${flowmeterData.errorCode.toString(16)}`);
        console.log(`   ⚖️  Mass Flow Rate: ${flowmeterData.massFlowRate.toFixed(3)} kg/h`);
        console.log(`   🌡️  Temperature: ${flowmeterData.temperature.toFixed(2)} °C`);
        console.log(`   💧 Volume Flow Rate: ${flowmeterData.volumeFlowRate.toFixed(3)} m³/h`);

        return flowmeterData;

      } catch (parseError) {
        console.error(`❌ Failed to parse flowmeter data:`, parseError);
        return null;
      }

    } catch (error) {
      console.error(`❌ Failed to read flowmeter data from device ${deviceAddress}:`, error);
      return null;
    }
  }

  /**
   * Enhanced device scanning with robust error handling
   */
  async scanForDevices(addressRange: number[] = [1, 2, 3, 4, 5]): Promise<number[]> {
    console.log(`🔍 Scanning for devices on addresses: [${addressRange.join(', ')}]`);
    const responsiveDevices: number[] = [];
    const scanResults: { address: number; responsive: boolean; error?: string }[] = [];

    for (const address of addressRange) {
      console.log(`\n🔍 Scanning device ${address}...`);
      
      try {
        const isResponsive = await this.testDeviceResponsive(address);
        scanResults.push({ address, responsive: isResponsive });
        
        if (isResponsive) {
          responsiveDevices.push(address);
          console.log(`✅ Device ${address} added to responsive list`);
        }
        
      } catch (error) {
        const errorMsg = error instanceof Error ? error.message : String(error);
        scanResults.push({ address, responsive: false, error: errorMsg });
        console.log(`❌ Device ${address} scan failed: ${errorMsg}`);
      }
      
      // Delay between device scans to avoid bus overload
      await new Promise(resolve => setTimeout(resolve, 500));
    }

    // Summary
    console.log(`\n📊 Scan Summary:`);
    console.log(`   ✅ Responsive devices: [${responsiveDevices.join(', ')}]`);
    console.log(`   📵 Non-responsive: [${addressRange.filter(addr => !responsiveDevices.includes(addr)).join(', ')}]`);
    console.log(`   📈 Success rate: ${((responsiveDevices.length / addressRange.length) * 100).toFixed(1)}%`);

    return responsiveDevices;
  }

  /**
   * Enhanced continuous monitoring with error recovery
   */
  async monitorFlowmeters(deviceAddresses: number[], intervalMs: number = 5000): Promise<void> {
    console.log(`🔄 Starting enhanced flowmeter monitoring`);
    console.log(`   📡 Devices: [${deviceAddresses.join(', ')}]`);
    console.log(`   ⏱️  Update interval: ${intervalMs}ms`);
    console.log(`   🛑 Press Ctrl+C to stop\n`);

    // Initial device scan
    const responsiveDevices = await this.scanForDevices(deviceAddresses);
    
    if (responsiveDevices.length === 0) {
      console.log('❌ No responsive devices found. Check connections and device addresses.');
      return;
    }

    let monitoringActive = true;
    let successfulReads = 0;
    let failedReads = 0;

    const monitor = setInterval(async () => {
      if (!monitoringActive) return;

      console.log(`\n⏰ ${new Date().toISOString()} - Reading flowmeter data...`);
      
      for (const deviceAddress of responsiveDevices) {
        try {
          const data = await this.readFlowmeterData(deviceAddress);
          
          if (data) {
            successfulReads++;
            console.log(`📊 Device ${deviceAddress} - Sealand Flowmeter:`);
            console.log(`   🏷️  Error Code: 0x${data.errorCode.toString(16)}`);
            console.log(`   ⚖️  Mass Flow: ${data.massFlowRate.toFixed(2)} kg/h`);
            console.log(`   🌡️  Temperature: ${data.temperature.toFixed(2)} °C`);
            console.log(`   💧 Volume Flow: ${data.volumeFlowRate.toFixed(3)} m³/h`);
            console.log(`   📊 Mass Total: ${data.massTotal.toFixed(2)} kg`);
            console.log(`   📈 Volume Total: ${data.volumeTotal.toFixed(3)} m³`);
          } else {
            failedReads++;
            console.log(`📵 Device ${deviceAddress} - Read failed (${failedReads} total failures)`);
          }
        } catch (error) {
          failedReads++;
          console.error(`💥 Device ${deviceAddress} - Exception:`, error);
        }
      }

      // Display statistics
      const totalReads = successfulReads + failedReads;
      if (totalReads > 0) {
        const successRate = ((successfulReads / totalReads) * 100).toFixed(1);
        console.log(`📈 Success rate: ${successRate}% (${successfulReads}/${totalReads})`);
      }
      
    }, intervalMs);

    // Enhanced graceful shutdown
    const shutdown = () => {
      console.log('\n🛑 Stopping flowmeter monitor...');
      monitoringActive = false;
      clearInterval(monitor);
      
      console.log(`📊 Final Statistics:`);
      console.log(`   ✅ Successful reads: ${successfulReads}`);
      console.log(`   ❌ Failed reads: ${failedReads}`);
      
      this.disconnect();
      process.exit(0);
    };

    process.on('SIGINT', shutdown);
    process.on('SIGTERM', shutdown);
  }

  /**
   * Debug helper for testing CRC implementation
   */
  testCRCImplementation(): void {
    console.log('🔧 Testing CRC implementation...');
    
    // Test with known Modbus frame
    const testData = Buffer.from([0x01, 0x03, 0x00, 0xF4, 0x00, 0x01]);
    const calculatedCrc = this.calculateCRC16(testData);
    console.log(`   📊 Test data: [${Array.from(testData).map(b => `0x${b.toString(16).padStart(2, '0')}`).join(', ')}]`);
    console.log(`   🔢 Calculated CRC: 0x${calculatedCrc.toString(16).padStart(4, '0')}`);
    console.log(`   📋 CRC bytes: Low=0x${(calculatedCrc & 0xFF).toString(16).padStart(2, '0')}, High=0x${((calculatedCrc >> 8) & 0xFF).toString(16).padStart(2, '0')}`);

    // Test frame creation and verification
    const testFrame = this.createReadHoldingRegistersFrame(1, 244, 1);
    console.log(`   🏗️  Created frame: [${Array.from(testFrame).map(b => `0x${b.toString(16).padStart(2, '0')}`).join(', ')}]`);
    
    const crcValid = this.verifyModbusCRC(testFrame);
    console.log(`   ✅ CRC verification: ${crcValid ? 'PASSED' : 'FAILED'}`);
  }

  /**
   * Set debug mode
   */
  setDebugMode(enabled: boolean): void {
    this.debugMode = enabled;
    console.log(`🔧 Debug mode ${enabled ? 'enabled' : 'disabled'}`);
  }
}

// Enhanced main function with comprehensive error handling
async function main() {
  const reader = new BL410ModbusReader();

  try {
    console.log('🖥️  BL410 Sealand Flowmeter Reader - Enhanced Version\n');
    
    // Test CRC implementation first
    reader.testCRCImplementation();
    console.log('');

    await BL410ModbusReader.listSerialPorts();
    console.log('');

    // Connect to RS485
    await reader.connect('/dev/ttyS0', 9600, 'even');

    // Device addresses to scan
    const flowmeterAddresses = [1, 2, 3, 4, 5];

    // Scan for responsive devices
    console.log('🔍 Scanning for responsive devices...\n');
    const responsiveDevices = await reader.scanForDevices(flowmeterAddresses);
    
    if (responsiveDevices.length === 0) {
      console.log('❌ No responsive devices found. Check your connections and device addresses.');
      reader.disconnect();
      return;
    }

    // Single read example
    console.log('\n📊 Single read example from responsive devices:\n');
    for (const address of responsiveDevices) {
      const data = await reader.readFlowmeterData(address);
      if (data) {
        console.log(`Flowmeter ${address}:`, {
          errorCode: `0x${data.errorCode.toString(16)}`,
          massFlowRate: data.massFlowRate.toFixed(3),
          temperature: data.temperature.toFixed(2),
          volumeTotal: data.volumeTotal.toFixed(3)
        });
      }
    }

    // Interactive monitoring option
    const readline = require('readline').createInterface({
      input: process.stdin,
      output: process.stdout
    });

    readline.question('\nStart continuous flowmeter monitoring? (y/n): ', (answer: string) => {
      readline.close();
      if (answer.toLowerCase() === 'y' || answer.toLowerCase() === 'yes') {
        reader.monitorFlowmeters(responsiveDevices, 10000);
      } else {
        reader.disconnect();
        console.log('👋 Goodbye!');
        process.exit(0);
      }
    });

  } catch (error) {
    console.error('💥 Main error:', error);
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
