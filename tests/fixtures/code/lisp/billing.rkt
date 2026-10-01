#lang racket/base

(require racket/list
         "util.rkt"
         (only-in racket/string string-join))

(struct invoice (id total))

(define limit 10)

(define (charge amount)
  (define tax 2)
  (+ amount tax))

(define ((adder n) m) (+ n m))

(define double (lambda (x) (* 2 x)))

(define-syntax-rule (twice body)
  (begin body body))

(module+ test
  (define (check) #t))
