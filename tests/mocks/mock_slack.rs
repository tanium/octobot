use std::sync::Arc;

use octobot::slack::SlackRequest;
use octobot::worker::Worker;

use mocks::mock_worker::LockedMockWorker;

pub struct MockSlack {
    worker: LockedMockWorker<SlackRequest>,
}

impl MockSlack {
    pub fn new(expected_calls: Vec<SlackRequest>) -> MockSlack {
        MockSlack { worker: LockedMockWorker::from_reqs("slack", expected_calls) }
    }

    pub fn expect(&mut self, reqs: Vec<SlackRequest>) {
        for req in reqs {
            self.worker.expect_req(req);
        }
    }

    pub fn new_sender(&self) -> Arc<dyn Worker<SlackRequest>> {
        self.worker.new_sender()
    }
}
