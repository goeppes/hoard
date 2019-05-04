pipeline {
  agent {
    docker {
      image 'rust:1.34'
      args "-v $WORKSPACE/target:/target"
    }
  }

  options {
    disableConcurrentBuilds()
  }

  stages {
    stage('Build') {
      sh "cargo build"
    }
  }

  post {
    always {
      archiveArtifacts artifacts: 'target/debug/hoard', onlyIfSuccessful: true
      cleanWs()
    }
  }
}
