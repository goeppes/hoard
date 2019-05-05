pipeline {
  agent {
    docker {
      image 'rust:1.34'
    }
  }

  options {
    disableConcurrentBuilds()
  }

  stages {
    stage('Build') {
      steps {
        sh "cargo build"
      }
    }
  }

  post {
    success {
      archiveArtifacts artifacts: 'target/debug/hoard', fingerprint: true
    }
    cleanup {
      cleanWs()
    }
  }
}
