use crate::storager::Storager;
use common::rpc::{
    storager_service_server::StoragerService, StoragerAddRequest, StoragerAddResponse,
    StoragerDeleteRequest, StoragerDeleteResponse, StoragerQueryRequest, StoragerQueryResponse,
};
use std::time::Instant;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl StoragerService for Storager {
    async fn add(
        &self,
        request: Request<StoragerAddRequest>,
    ) -> Result<Response<StoragerAddResponse>, Status> {
        let req = request.into_inner();
        println!(
            "Storager received Add request: keyword={}, fid={}",
            req.keyword, req.fid
        );

        let mut ads = self.ads.write()
            .expect("Failed to acquire write lock on ads");
        
        let start = Instant::now();
        let (proof, root_hash) = ads.add(&req.keyword, &req.fid);
        let duration = start.elapsed();
        println!("[METRIC] Proof Generation (Add): {:?}", duration);

        Ok(Response::new(StoragerAddResponse { proof, root_hash }))
    }

    async fn query(
        &self,
        request: Request<StoragerQueryRequest>,
    ) -> Result<Response<StoragerQueryResponse>, Status> {
        let req = request.into_inner();
        println!("Storager received Query request: keyword={}", req.keyword);

        let ads = self.ads.read()
            .expect("Failed to acquire read lock on ads");
        
        let start = Instant::now();
        let (fids, proof) = ads.query(&req.keyword);
        let duration = start.elapsed();
        println!("[METRIC] Proof Generation (Query): {:?}", duration);

        Ok(Response::new(StoragerQueryResponse { fids, proof }))
    }

    async fn delete(
        &self,
        request: Request<StoragerDeleteRequest>,
    ) -> Result<Response<StoragerDeleteResponse>, Status> {
        let req = request.into_inner();
        println!(
            "Storager received Delete request: keyword={}, fid={}",
            req.keyword, req.fid
        );

        let mut ads = match self.ads.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                eprintln!("⚠️ Delete service: recovering from poisoned Mutex");
                poisoned.into_inner()
            }
        };
        
        let start = Instant::now();
        let (proof, root_hash) = ads.delete(&req.keyword, &req.fid);
        let duration = start.elapsed();
        println!("[METRIC] Proof Generation (Delete): {:?}", duration);

        Ok(Response::new(StoragerDeleteResponse { proof, root_hash }))
    }
}
