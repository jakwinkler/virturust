from flask import Flask
app = Flask(__name__)

@app.route('/')
def hello():
    return '<h1>Hello from Corten!</h1><p>Python + Flask, no Docker.</p>'

if __name__ == '__main__':
    app.run(host='0.0.0.0', port=5000)
