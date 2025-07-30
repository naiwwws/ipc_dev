use actix_web::{web, App, HttpServer, HttpResponse, Result as ActixResult, middleware::Logger};
use log::{info, error, warn};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::sync::Arc;

use crate::config::Config;
use crate::utils::error::ModbusError;
use crate::storage::{SqliteManager, Transaction};
use crate::services::data_service::DataService; // FIXED: Updated import path

// Transaction flow types
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum FlowType {
    FlowIn,
    FlowOut,
}

// Transaction confirmation request payload
#[derive(Debug, Deserialize, Serialize)]
pub struct NewTransactionRequest {
    // Flow type
    pub flow_type: FlowType,
    
    // Vessel data
    pub vessel_id: String,
    pub vessel_name: String,
    pub vessel_type: String,
    pub liquid_target_volume: f32,
    
    // Liquid information
    pub liquid_type: String,
    pub liquid_density_min: f32,
    pub liquid_density_max: f32,
    pub liquid_water_content_min: f32,
    pub liquid_water_content_max: f32,
    pub liquid_residual_carbon_min: f32,
    pub liquid_residual_carbon_max: f32,
    
    // Auth session
    pub operator_full_name: String,
    pub operator_email: String,
    pub operator_phone_number: Option<String>,
    
    // Customer information (for flow_out)
    pub customer_vessel_name: Option<String>,
    pub customer_pic_name: Option<String>,
    pub customer_location_name: Option<String>,
    
    // Supplier information (for flow_in)
    pub supplier_name: Option<String>,
}

// Transaction response
#[derive(Debug, Serialize)]
pub struct TransactionResponse {
    pub success: bool,
    pub transaction_id: String,
    pub message: String,
    pub timestamp: DateTime<Utc>,
    pub flow_type: FlowType,
    pub vessel_id: String,
}

// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub success: bool,
    pub error: String,
    pub code: String,
    pub timestamp: DateTime<Utc>,
}

// API Service state
#[derive(Clone)]
pub struct ApiServiceState {
    pub sqlite_manager: SqliteManager,
    pub config: Config,
    pub data_service: Option<Arc<DataService>>, // NEW field
}

impl ApiServiceState {
    pub fn new(config: Config, sqlite_manager: SqliteManager, data_service: Option<Arc<DataService>>) -> Self {
        Self {
            sqlite_manager,
            config,
            data_service,
        }
    }
}

// API Service
pub struct ApiService {
    state: ApiServiceState,
    server_handle: Option<actix_web::dev::ServerHandle>,
}

impl ApiService {
    pub fn new(config: Config, sqlite_manager: SqliteManager) -> Self {
        let state = ApiServiceState::new(config, sqlite_manager, None);
        Self {
            state,
            server_handle: None,
        }
    }
    
    // NEW: Constructor that accepts pre-built state with DataService
    pub fn new_with_state(state: ApiServiceState) -> Result<Self, ModbusError> {
        Ok(Self {
            state,
            server_handle: None,
        })
    }

    pub async fn start(&mut self, port: u16) -> Result<(), ModbusError> {
        info!("🌐 Starting HTTP API server on port {}", port);
        
        let state_data = web::Data::new(self.state.clone());
        
        let server = HttpServer::new(move || {
            App::new()
                .app_data(state_data.clone())
                .wrap(Logger::default())
                .service(
                    web::scope("/api")
                        .route("/health", web::get().to(health_check))
                        .route("/new_trx_confirm", web::post().to(create_new_transaction))
                        .route("/overview/trx_report", web::get().to(list_transactions))
                        .route("/overview/trx_report/{id}", web::get().to(get_transaction))
                )
        })
        .bind(format!("0.0.0.0:{}", port))?
        .run();
        
        // Store server handle for graceful shutdown
        self.server_handle = Some(server.handle());
        
        // Start the server in background
        tokio::spawn(async move {
            if let Err(e) = server.await {
                error!("❌ HTTP API server error: {}", e);
            }
        });
        
        info!("✅ HTTP API server started successfully on port {}", port);
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), ModbusError> {
        info!("🛑 Stopping HTTP API server...");
        
        if let Some(handle) = self.server_handle.take() {
            // Use graceful shutdown with timeout
            tokio::select! {
                _ = handle.stop(true) => {
                    info!("✅ HTTP API server stopped gracefully");
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {
                    warn!("⚠️  HTTP API server shutdown timeout, forcing stop");
                    handle.stop(false).await;
                }
            }
        }
        
        Ok(())
    }

    pub async fn get_active_transactions(&self) -> Result<Vec<Transaction>, ModbusError> {
        self.state.sqlite_manager.get_all_transactions(Some(100), None).await
    }
}

// API Endpoints

// POST /api/new_trx_confirm - Create new transaction
async fn create_new_transaction(
    request: web::Json<NewTransactionRequest>,
    state: web::Data<ApiServiceState>,
) -> ActixResult<HttpResponse> {
    info!("📝 Received new transaction request for vessel: {}", request.vessel_name);
    
    // Validate request
    if let Err(validation_error) = validate_transaction_request(&request) {
        warn!("❌ Transaction validation failed: {}", validation_error);
        return Ok(HttpResponse::BadRequest().json(ErrorResponse {
            success: false,
            error: validation_error,
            code: "VALIDATION_ERROR".to_string(),
            timestamp: Utc::now(),
        }));
    }

    // Generate transaction ID
    let transaction_id = Uuid::new_v4().to_string();
    
    // Create transaction for database
    let transaction = Transaction::new(
        transaction_id.clone(),
        format!("{:?}", request.flow_type).to_lowercase(),
        request.vessel_id.clone(),
        request.vessel_name.clone(),
        request.vessel_type.clone(),
        request.liquid_target_volume,
        request.liquid_type.clone(),
        request.liquid_density_min,
        request.liquid_density_max,
        request.liquid_water_content_min,
        request.liquid_water_content_max,
        request.liquid_residual_carbon_min,
        request.liquid_residual_carbon_max,
        request.operator_full_name.clone(),
        request.operator_email.clone(),
        request.operator_phone_number.clone(),
        request.customer_vessel_name.clone(),
        request.customer_pic_name.clone(),
        request.customer_location_name.clone(),
        request.supplier_name.clone(),
        "confirmed".to_string(),
    );

    // Store transaction in database
    match state.sqlite_manager.insert_transaction(&transaction).await {
        Ok(_) => {
            info!("✅ Transaction created in database: {} for vessel: {}", 
                  transaction_id, request.vessel_name);

            // CRITICAL FIX: Actually start volume tracking in DataService
            if let Some(data_service) = &state.data_service {
                info!("🔗 Starting volume tracking for transaction: {}", transaction_id);
                data_service.start_transaction_with_volume(
                    transaction_id.clone(),
                    request.liquid_target_volume,
                    request.vessel_name.clone()
                ).await;
                info!("🎯 Volume tracking started for transaction: {} (target: {:.2} L)", 
                      transaction_id, request.liquid_target_volume);
            } else {
                warn!("⚠️ DataService not available - transaction created but volume tracking disabled");
                warn!("💡 Transaction will be stored in database but won't trigger completion automatically");
            }

            Ok(HttpResponse::Ok().json(TransactionResponse {
                success: true,
                transaction_id,
                message: format!("Transaction confirmed for vessel: {} (target volume: {:.2} L)", 
                               request.vessel_name, request.liquid_target_volume),
                timestamp: Utc::now(),
                flow_type: request.flow_type.clone(),
                vessel_id: request.vessel_id.clone(),
            }))
        },
        Err(e) => {
            error!("❌ Failed to store transaction: {}", e);
            Ok(HttpResponse::InternalServerError().json(ErrorResponse {
                success: false,
                error: "Failed to store transaction".to_string(),
                code: "DATABASE_ERROR".to_string(),
                timestamp: Utc::now(),
            }))
        }
    }
}

// GET /api/health - Health check
async fn health_check() -> ActixResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "service": "IPC API Service",
        "timestamp": Utc::now(),
        "version": crate::VERSION
    })))
}

// GET /api/overview/trx_report - List all transactions
async fn list_transactions(
    state: web::Data<ApiServiceState>,
) -> ActixResult<HttpResponse> {
    match state.sqlite_manager.get_all_transactions(Some(100), None).await {
        Ok(transactions) => {
            let transaction_summaries: Vec<serde_json::Value> = transactions
                .iter()
                .map(|t| serde_json::json!({
                    "id": t.transaction_id,
                    "flow_type": t.flow_type,
                    "vessel_name": t.vessel_name,
                    "vessel_id": t.vessel_id,
                    "liquid_target_volume": t.liquid_target_volume,
                    "operator_full_name": t.operator_full_name,
                    "created_at": t.created_at,
                    "status": t.status
                }))
                .collect();

            Ok(HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "count": transaction_summaries.len(),
                "transactions": transaction_summaries,
                "timestamp": Utc::now()
            })))
        },
        Err(e) => {
            error!("❌ Failed to fetch transactions: {}", e);
            Ok(HttpResponse::InternalServerError().json(ErrorResponse {
                success: false,
                error: "Failed to fetch transactions".to_string(),
                code: "DATABASE_ERROR".to_string(),
                timestamp: Utc::now(),
            }))
        }
    }
}

// GET /api/overview/trx_report/{id} - Get specific transaction
async fn get_transaction(
    path: web::Path<String>,
    state: web::Data<ApiServiceState>,
) -> ActixResult<HttpResponse> {
    let transaction_id = path.into_inner();
    
    match state.sqlite_manager.get_transaction_by_id(&transaction_id).await {
        Ok(Some(transaction)) => {
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "transaction": transaction,
                "timestamp": Utc::now()
            })))
        },
        Ok(None) => {
            Ok(HttpResponse::NotFound().json(ErrorResponse {
                success: false,
                error: format!("Transaction not found: {}", transaction_id),
                code: "TRANSACTION_NOT_FOUND".to_string(),
                timestamp: Utc::now(),
            }))
        },
        Err(e) => {
            error!("❌ Failed to fetch transaction: {}", e);
            Ok(HttpResponse::InternalServerError().json(ErrorResponse {
                success: false,
                error: "Failed to fetch transaction".to_string(),
                code: "DATABASE_ERROR".to_string(),
                timestamp: Utc::now(),
            }))
        }
    }
}

// Helper function to validate transaction request
fn validate_transaction_request(request: &NewTransactionRequest) -> Result<(), String> {
    // Validate vessel ID format (should be UUID)
    if Uuid::parse_str(&request.vessel_id).is_err() {
        return Err("Invalid vessel_id format, should be UUID".to_string());
    }

    // Validate email format
    if !request.operator_email.contains('@') {
        return Err("Invalid operator email format".to_string());
    }

    // Validate liquid target volume
    if request.liquid_target_volume <= 0.0 {
        return Err("Liquid target volume must be greater than 0".to_string());
    }

    // Validate density ranges
    if request.liquid_density_min >= request.liquid_density_max {
        return Err("Liquid density min must be less than max".to_string());
    }

    // Validate water content ranges
    if request.liquid_water_content_min >= request.liquid_water_content_max {
        return Err("Liquid water content min must be less than max".to_string());
    }

    // Validate residual carbon ranges
    if request.liquid_residual_carbon_min >= request.liquid_residual_carbon_max {
        return Err("Liquid residual carbon min must be less than max".to_string());
    }

    // Validate flow type specific fields
    match request.flow_type {
        FlowType::FlowOut => {
            if request.customer_vessel_name.is_none() || 
               request.customer_pic_name.is_none() || 
               request.customer_location_name.is_none() {
                return Err("Customer information required for flow_out transactions".to_string());
            }
        },
        FlowType::FlowIn => {
            if request.supplier_name.is_none() {
                return Err("Supplier name required for flow_in transactions".to_string());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_transaction_validation() {
        let valid_request = NewTransactionRequest {
            flow_type: FlowType::FlowIn,
            vessel_id: "54da7c65-48ed-4375-95e3-d89e6b6cecca".to_string(),
            vessel_name: "Test Vessel".to_string(),
            vessel_type: "Chemical Tanker".to_string(),
            liquid_target_volume: 10000.0,
            liquid_type: "Mid Sulfur Fuel OIL 1223".to_string(),
            liquid_density_min: 10.0,
            liquid_density_max: 100.0,
            liquid_water_content_min: 5.0,
            liquid_water_content_max: 30.0,
            liquid_residual_carbon_min: 5.0,
            liquid_residual_carbon_max: 20.0,
            operator_full_name: "Test Operator".to_string(),
            operator_email: "test@example.com".to_string(),
            operator_phone_number: Some("+1234567890".to_string()),
            customer_vessel_name: None,
            customer_pic_name: None,
            customer_location_name: None,
            supplier_name: Some("Test Supplier".to_string()),
        };

        assert!(validate_transaction_request(&valid_request).is_ok());
    }
}