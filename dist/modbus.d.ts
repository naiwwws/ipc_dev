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
declare class BL410ModbusReader {
    private serialPort;
    private isConnected;
    private devices;
    private flowmeterRegOffset;
    private responseBuffer;
    constructor();
    /**
     * Initialize RS485 connection with raw serial port
     * @param port Serial port (e.g., '/dev/ttyS0' for BL410)
     * @param baudRate Baud rate (default: 9600)
     * @param parity Parity setting (default: 'even')
     */
    connect(port?: string, baudRate?: number, parity?: 'even' | 'none' | 'odd'): Promise<void>;
    /**
     * Disconnect from RS485
     */
    disconnect(): void;
    /**
     * List available serial ports
     */
    static listSerialPorts(): Promise<void>;
    /**
     * Convert 32-bit integer to IEEE 754 float
     * Mimics the Lua string.unpack("f", string.pack("i4", value))
     */
    private int32ToFloat;
    /**
     * Calculate CRC-16 Modbus checksum
     * Standard Modbus CRC-16 implementation
     * @param data Buffer containing data to calculate CRC for
     * @returns CRC-16 value
     */
    private calculateCRC16;
    /**
     * Create Modbus Read Holding Registers frame with CRC
     * @param address Device address
     * @param startRegisterAddr Starting register address
     * @param quantity Number of registers to read
     * @returns Complete Modbus frame with CRC
     */
    private createReadHoldingRegistersFrame;
    /**
     * Verify Modbus frame CRC
     * @param frameData Complete frame data including CRC bytes
     * @returns true if CRC is valid
     */
    private verifyModbusCRC;
    /**
     * Send raw Modbus frame and wait for response
     */
    private sendRawModbusFrame;
    /**
     * Read flowmeter data using CRC validation
     */
    readFlowmeterData(deviceAddress: number): Promise<FlowmeterData | null>;
    /**
     * Test if a device is responsive using raw communication
     * @param deviceAddress Device address to test
     * @returns true if device responds, false otherwise
     */
    testDeviceResponsive(deviceAddress: number): Promise<boolean>;
    /**
     * Scan for available devices on the bus
     * @param addressRange Array of addresses to scan
     * @returns Array of responsive device addresses
     */
    scanForDevices(addressRange?: number[]): Promise<number[]>;
    /**
     * Monitor multiple flowmeters continuously
     */
    monitorFlowmeters(deviceAddresses: number[], intervalMs?: number): Promise<void>;
}
export { BL410ModbusReader, ModbusDevice, FlowmeterData };
//# sourceMappingURL=modbus.d.ts.map