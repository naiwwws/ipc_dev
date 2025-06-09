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
    private client;
    private isConnected;
    private devices;
    private flowmeterRegOffset;
    constructor();
    /**
     * Initialize RS485 connection
     * @param port Serial port (e.g., '/dev/ttyS0' for BL410)
     * @param baudRate Baud rate (default: 9600)
     * @param parity Parity setting (default: 'none')
     */
    connect(port?: string, baudRate?: number, parity?: 'none' | 'even' | 'odd'): Promise<void>;
    /**
     * Convert two 16-bit registers to a 32-bit integer (Big Endian)
     * Mimics the Lua bit manipulation: (arg[i] << 8 | arg[i+1]) << 16 | (arg[i+2] << 8 | arg[i+3])
     */
    private registers32ToInt;
    /**
     * Convert 32-bit integer to IEEE 754 float
     * Mimics the Lua string.unpack("f", string.pack("i4", value))
     */
    private int32ToFloat;
    /**
     * Read flowmeter data from Sealand flowmeter
     * Based on the Lua implementation in tt_flowmeter_sealand service
     */
    readFlowmeterData(deviceAddress: number): Promise<FlowmeterData | null>;
    /**
     * Read complete flowmeter data (all registers)
     * Reads additional registers to get complete data set
     */
    readCompleteFlowmeterData(deviceAddress: number): Promise<FlowmeterData | null>;
    /**
     * Reset flowmeter accumulation
     * Mimics the resetAccumulation command from the Lua service
     */
    resetFlowmeterAccumulation(deviceAddress: number): Promise<boolean>;
    /**
     * Monitor multiple flowmeters continuously
     */
    monitorFlowmeters(deviceAddresses: number[], intervalMs?: number): Promise<void>;
    /**
     * Add a Modbus device configuration
     */
    addDevice(device: ModbusDevice): void;
    /**
     * Read holding registers from a Modbus device
     */
    readHoldingRegisters(deviceId: number, address: number, count: number): Promise<number[]>;
    /**
     * Read input registers from a Modbus device
     */
    readInputRegisters(deviceId: number, address: number, count: number): Promise<number[]>;
    /**
     * Read coils from a Modbus device
     */
    readCoils(deviceId: number, address: number, count: number): Promise<boolean[]>;
    /**
     * Write single holding register
     */
    writeSingleRegister(deviceId: number, address: number, value: number): Promise<void>;
    /**
     * List available serial ports (useful for debugging)
     */
    static listSerialPorts(): Promise<void>;
    /**
     * Disconnect from RS485
     */
    disconnect(): void;
}
export { BL410ModbusReader, ModbusDevice, FlowmeterData };
//# sourceMappingURL=modbus.d.ts.map