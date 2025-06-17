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
    private debugMode;
    constructor();
    /**
     * Initialize RS485 connection with enhanced error handling
     */
    connect(port?: string, baudRate?: number, parity?: 'even' | 'none' | 'odd'): Promise<void>;
    /**
     * Disconnect from RS485 with cleanup
     */
    disconnect(): void;
    /**
     * Enhanced serial port listing
     */
    static listSerialPorts(): Promise<void>;
    /**
     * Convert 32-bit integer to IEEE 754 float (Lua compatible)
     */
    private int32ToFloat;
    /**
     * Enhanced CRC-16 Modbus calculation with validation
     */
    private calculateCRC16;
    /**
     * Create Modbus Read Holding Registers frame with CONSISTENT CRC byte order
     */
    private createReadHoldingRegistersFrame;
    /**
     * Enhanced CRC verification with CONSISTENT byte order handling
     */
    private verifyModbusCRC;
    /**
     * Enhanced raw Modbus frame sender with dynamic response detection
     */
    private sendRawModbusFrame;
    /**
     * Enhanced device responsiveness test with multiple attempts
     */
    testDeviceResponsive(deviceAddress: number): Promise<boolean>;
    /**
     * Enhanced flowmeter data reading with comprehensive error handling
     */
    readFlowmeterData(deviceAddress: number): Promise<FlowmeterData | null>;
    /**
     * Enhanced device scanning with robust error handling
     */
    scanForDevices(addressRange?: number[]): Promise<number[]>;
    /**
     * Enhanced continuous monitoring with error recovery
     */
    monitorFlowmeters(deviceAddresses: number[], intervalMs?: number): Promise<void>;
    /**
     * Debug helper for testing CRC implementation
     */
    testCRCImplementation(): void;
    /**
     * Set debug mode
     */
    setDebugMode(enabled: boolean): void;
}
export { BL410ModbusReader, ModbusDevice, FlowmeterData };
//# sourceMappingURL=modbus.d.ts.map