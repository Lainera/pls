PHOHY: img

BIN?=spawn
workdir := $(shell dirname $(abspath $(lastword $(MAKEFILE_LIST))))

img: 
	docker build --build-arg BIN=$(BIN) -t $(BIN):latest .

outdir:
	mkdir -p $(workdir)/out 

bin: outdir img 
	docker run --rm --entrypoint cp -v $(workdir)/out:/tmp/out $(BIN):latest /usr/local/bin/$(BIN) /tmp/out/$(BIN)
