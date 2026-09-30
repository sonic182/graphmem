package main

import "fmt"

type Server struct {
	Addr string
}

func (s *Server) Start() error {
	return nil
}

type Handler interface {
	Serve(path string) error
}

func main() {
	fmt.Println("start")
}
