#!/usr/bin/env bash

# save workspace dir
WORKSPACE=$(pwd)

# Create a temp directory
TEMPDIR=/tmp/setup
mkdir -p $TEMPDIR

# This will be shown in the dev container log 
# See it in VSCode using "Dev Containers: Show container log"
echo "Executing postCreate script, workspace is ${WORKSPACE}"

# These commands will be executed once after container build