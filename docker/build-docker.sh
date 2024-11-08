#!/bin/bash

PACKAGE_TAG="ghcr.io/clownpiecestripedabyss/wanna-cdn"

if git rev-parse --git-dir > /dev/null 2>&1; then
    if [[ $NO_DIRTY != '' ]]; then
        PACKAGE_VER=`git describe --always`
    else
        PACKAGE_VER=`git describe --always --dirty`
        if [[ $(git status --porcelain) != '' ]]; then
            PACKAGE_VER="${PACKAGE_VER}-untracked"
        fi
    fi
else
  PACKAGE_VER="non-git-tree"
fi

PACKAGE_TAG_FULL="$PACKAGE_TAG:$PACKAGE_VER"
PACKAGE_TAG_LATEST="$PACKAGE_TAG:latest"

echo "Building $PACKAGE_TAG_FULL"
docker build . -t $PACKAGE_TAG_FULL -f docker/aya-dance/Dockerfile

echo "Tagged $PACKAGE_TAG_FULL as $PACKAGE_TAG_LATEST"
docker tag $PACKAGE_TAG_FULL $PACKAGE_TAG_LATEST

if [[ $NO_PUSH != '' ]]; then
    echo $NO_PUSH
else
    echo "Pushing $PACKAGE_TAG_FULL"
    docker push $PACKAGE_TAG_FULL
    echo "Pushing $PACKAGE_TAG_FULL as $PACKAGE_TAG_LATEST"
    docker push $PACKAGE_TAG_LATEST
fi
